//! Tests for the SSE event transformer (`web::sse`).
//!
//! Each test constructs an `AgentEventEnvelope` with a specific `AgentEvent`
//! variant, passes it through `transform_event()`, and asserts on the resulting
//! JSON payload (or `None` for skipped events).

use agent_sdk::{AgentEvent, AgentEventEnvelope, SequenceCounter, TokenUsage, ToolResult, ToolTier};
use claw_agent_rs::web::sse::{estimate_cost, transform_event};
use serde_json::json;
use std::time::Duration;

/// Helper: wrap an `AgentEvent` in an envelope for testing.
fn envelope(event: AgentEvent) -> AgentEventEnvelope {
    let seq = SequenceCounter::new();
    AgentEventEnvelope::wrap(event, &seq)
}

// ═══════════════════════════════════════════════════════════════════════════
// Events that produce Some(json)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn text_delta_event() {
    let env = envelope(AgentEvent::text_delta("msg_1", "hello"));
    let result = transform_event(&env).expect("TextDelta should produce Some");

    assert_eq!(result["type"], "text_delta");
    assert_eq!(result["text"], "hello");
}

#[test]
fn thinking_delta_event() {
    let env = envelope(AgentEvent::thinking_delta("msg_1", "let me think..."));
    let result = transform_event(&env).expect("ThinkingDelta should produce Some");

    assert_eq!(result["type"], "thinking");
    assert_eq!(result["text"], "let me think...");
}

#[test]
fn tool_call_start_event() {
    let input = json!({"command": "ls -la"});
    let env = envelope(AgentEvent::tool_call_start(
        "tool_42",
        "bash",
        "Bash",
        input.clone(),
        ToolTier::Observe,
    ));
    let result = transform_event(&env).expect("ToolCallStart should produce Some");

    assert_eq!(result["type"], "tool_use_start");
    assert_eq!(result["id"], "tool_42");
    assert_eq!(result["name"], "bash");
    // input is serialized to a JSON string
    let input_str: String = serde_json::from_value(result["input"].clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&input_str).unwrap();
    assert_eq!(parsed, input);
}

#[test]
fn tool_call_end_event() {
    let result_ok = ToolResult::success("file listing output");
    let env = envelope(AgentEvent::tool_call_end(
        "tool_42",
        "bash",
        "Bash",
        result_ok,
    ));
    let result = transform_event(&env).expect("ToolCallEnd should produce Some");

    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["id"], "tool_42");
    assert_eq!(result["output"], "file listing output");
    assert_eq!(result["is_error"], false);
}

#[test]
fn tool_call_end_error_event() {
    let result_err = ToolResult::error("command failed");
    let env = envelope(AgentEvent::tool_call_end(
        "tool_99",
        "bash",
        "Bash",
        result_err,
    ));
    let result = transform_event(&env).expect("ToolCallEnd (error) should produce Some");

    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["id"], "tool_99");
    assert_eq!(result["output"], "command failed");
    assert_eq!(result["is_error"], true);
}

#[test]
fn error_event() {
    let env = envelope(AgentEvent::error("something went wrong", false));
    let result = transform_event(&env).expect("Error should produce Some");

    assert_eq!(result["type"], "error");
    assert_eq!(result["error"], "something went wrong");
}

#[test]
fn done_event() {
    let usage = TokenUsage {
        input_tokens: 1000,
        output_tokens: 500,
    };
    let thread_id = agent_sdk::ThreadId::from_string("thread_abc");
    let env = envelope(AgentEvent::done(thread_id, 3, usage, Duration::from_millis(1234)));
    let result = transform_event(&env).expect("Done should produce Some");

    assert_eq!(result["type"], "done");
    assert_eq!(result["num_turns"], 3);
    assert_eq!(result["duration_ms"], 1234);
    assert_eq!(result["input_tokens"], 1000);
    assert_eq!(result["output_tokens"], 500);
    // cost_usd should be present and non-negative
    let cost = result["cost_usd"].as_f64().unwrap();
    assert!(cost >= 0.0);
    // Verify exact cost: (1000 * 3 / 1M) + (500 * 15 / 1M) = 0.003 + 0.0075 = 0.0105
    assert!((cost - 0.0105).abs() < 1e-9);
}

#[test]
fn tool_progress_event() {
    let env = envelope(AgentEvent::tool_progress(
        "tool_7",
        "bash",
        "Bash",
        "running",
        "executing command",
        None,
    ));
    let result = transform_event(&env).expect("ToolProgress should produce Some");

    assert_eq!(result["type"], "tool_progress");
    assert_eq!(result["tool_use_id"], "tool_7");
    assert_eq!(result["tool_name"], "bash");
}

#[test]
fn subagent_progress_started_event() {
    let env = envelope(AgentEvent::SubagentProgress {
        subagent_id: "sub_1".to_string(),
        subagent_name: "explorer".to_string(),
        tool_name: "grep".to_string(),
        tool_context: "searching for pattern".to_string(),
        completed: false,
        success: false,
        tool_count: 3,
        total_tokens: 500,
    });
    let result = transform_event(&env).expect("SubagentProgress (started) should produce Some");

    assert_eq!(result["type"], "sub_tool_use_start");
    assert_eq!(result["id"], "sub_1_3");
    assert_eq!(result["name"], "grep");
    assert_eq!(result["input"], "");
    assert_eq!(result["parent_tool_use_id"], "sub_1");
}

#[test]
fn subagent_progress_completed_event() {
    let env = envelope(AgentEvent::SubagentProgress {
        subagent_id: "sub_2".to_string(),
        subagent_name: "planner".to_string(),
        tool_name: "read".to_string(),
        tool_context: "file content here".to_string(),
        completed: true,
        success: true,
        tool_count: 5,
        total_tokens: 1200,
    });
    let result = transform_event(&env).expect("SubagentProgress (completed) should produce Some");

    assert_eq!(result["type"], "sub_tool_result");
    assert_eq!(result["id"], "sub_2_5");
    assert_eq!(result["output"], "file content here");
    assert_eq!(result["is_error"], false);
    assert_eq!(result["parent_tool_use_id"], "sub_2");
}

#[test]
fn subagent_progress_completed_failure_event() {
    let env = envelope(AgentEvent::SubagentProgress {
        subagent_id: "sub_3".to_string(),
        subagent_name: "worker".to_string(),
        tool_name: "bash".to_string(),
        tool_context: "command not found".to_string(),
        completed: true,
        success: false,
        tool_count: 1,
        total_tokens: 200,
    });
    let result = transform_event(&env).expect("SubagentProgress (failure) should produce Some");

    assert_eq!(result["type"], "sub_tool_result");
    assert_eq!(result["is_error"], true);
}

// ═══════════════════════════════════════════════════════════════════════════
// Events that are skipped (return None)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn start_event_skipped() {
    let thread_id = agent_sdk::ThreadId::from_string("t1");
    let env = envelope(AgentEvent::start(thread_id, 1));
    assert!(transform_event(&env).is_none(), "Start should be skipped");
}

#[test]
fn text_event_skipped() {
    let env = envelope(AgentEvent::text("msg_1", "complete text"));
    assert!(transform_event(&env).is_none(), "Text should be skipped");
}

#[test]
fn thinking_event_skipped() {
    let env = envelope(AgentEvent::thinking("msg_1", "full thinking"));
    assert!(
        transform_event(&env).is_none(),
        "Thinking should be skipped"
    );
}

#[test]
fn turn_complete_event_skipped() {
    let usage = TokenUsage {
        input_tokens: 100,
        output_tokens: 50,
    };
    let env = envelope(AgentEvent::TurnComplete { turn: 1, usage });
    assert!(
        transform_event(&env).is_none(),
        "TurnComplete should be skipped"
    );
}

#[test]
fn refusal_event_skipped() {
    let env = envelope(AgentEvent::refusal("msg_1", Some("policy violation".to_string())));
    assert!(
        transform_event(&env).is_none(),
        "Refusal should be skipped"
    );
}

#[test]
fn context_compacted_event_skipped() {
    let env = envelope(AgentEvent::context_compacted(50, 10, 8000, 2000));
    assert!(
        transform_event(&env).is_none(),
        "ContextCompacted should be skipped"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// estimate_cost
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn estimate_cost_zero_tokens() {
    assert!((estimate_cost(0, 0) - 0.0).abs() < 1e-12);
}

#[test]
fn estimate_cost_input_only() {
    // 1,000,000 input tokens at $3/M = $3.00
    let cost = estimate_cost(1_000_000, 0);
    assert!((cost - 3.0).abs() < 1e-9);
}

#[test]
fn estimate_cost_output_only() {
    // 1,000,000 output tokens at $15/M = $15.00
    let cost = estimate_cost(0, 1_000_000);
    assert!((cost - 15.0).abs() < 1e-9);
}

#[test]
fn estimate_cost_mixed_tokens() {
    // 500 input ($0.0015) + 200 output ($0.003) = $0.0045
    let cost = estimate_cost(500, 200);
    assert!((cost - 0.0045).abs() < 1e-9);
}

#[test]
fn estimate_cost_rounding() {
    // Verify rounding to 6 decimal places
    // 1 input token = $0.000003, 1 output token = $0.000015
    // total = $0.000018 → should be 0.000018 after round
    let cost = estimate_cost(1, 1);
    assert!((cost - 0.000018).abs() < 1e-9);
}

#[test]
fn estimate_cost_large_values() {
    // 10M input + 5M output
    // (10M * 3 / 1M) + (5M * 15 / 1M) = 30 + 75 = 105.0
    let cost = estimate_cost(10_000_000, 5_000_000);
    assert!((cost - 105.0).abs() < 1e-9);
}
