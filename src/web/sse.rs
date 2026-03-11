//! SSE event transformer — converts `AgentEventEnvelope` into the
//! JSON format expected by the SoulClaw frontend.

use agent_sdk::{AgentEvent, AgentEventEnvelope};
use serde_json::{json, Value};

/// Transform an agent-sdk event envelope into the frontend SSE event format.
/// Returns `None` for events that should be silently skipped.
pub fn transform_event(envelope: &AgentEventEnvelope) -> Option<Value> {
    match &envelope.event {
        AgentEvent::TextDelta { delta, .. } => Some(json!({
            "type": "text_delta",
            "text": delta,
        })),

        AgentEvent::ThinkingDelta { delta, .. } => Some(json!({
            "type": "thinking",
            "text": delta,
        })),

        AgentEvent::ToolCallStart { id, name, input, .. } => Some(json!({
            "type": "tool_use_start",
            "id": id,
            "name": name,
            "input": serde_json::to_string(input).unwrap_or_default(),
        })),

        AgentEvent::ToolCallEnd { id, result, .. } => Some(json!({
            "type": "tool_result",
            "id": id,
            "output": result.output,
            "is_error": !result.success,
        })),

        AgentEvent::ToolProgress { id, name, .. } => Some(json!({
            "type": "tool_progress",
            "tool_use_id": id,
            "tool_name": name,
            "parent_tool_use_id": null,
            "elapsed_seconds": 0,
        })),

        AgentEvent::SubagentProgress {
            subagent_id,
            tool_name,
            tool_context,
            completed,
            success,
            tool_count,
            ..
        } => {
            let sub_id = format!("{subagent_id}_{tool_count}");
            if *completed {
                Some(json!({
                    "type": "sub_tool_result",
                    "id": sub_id,
                    "output": tool_context,
                    "is_error": !success,
                    "parent_tool_use_id": subagent_id,
                }))
            } else {
                Some(json!({
                    "type": "sub_tool_use_start",
                    "id": sub_id,
                    "name": tool_name,
                    "input": "",
                    "parent_tool_use_id": subagent_id,
                }))
            }
        }

        AgentEvent::Done {
            total_turns,
            total_usage,
            duration,
            ..
        } => {
            let cost = estimate_cost(total_usage.input_tokens, total_usage.output_tokens);
            Some(json!({
                "type": "done",
                "result": null,
                "cost_usd": cost,
                "duration_ms": duration.as_millis() as u64,
                "num_turns": total_turns,
                "input_tokens": total_usage.input_tokens,
                "output_tokens": total_usage.output_tokens,
                "cache_read_tokens": 0,
                "cache_creation_tokens": 0,
            }))
        }

        AgentEvent::Error { message, .. } => Some(json!({
            "type": "error",
            "error": message,
        })),

        // Events we skip (not relevant to frontend)
        AgentEvent::Start { .. }
        | AgentEvent::Text { .. }
        | AgentEvent::Thinking { .. }
        | AgentEvent::TurnComplete { .. }
        | AgentEvent::ToolRequiresConfirmation { .. }
        | AgentEvent::Refusal { .. }
        | AgentEvent::ContextCompacted { .. } => None,
    }
}

/// Rough cost estimation based on token counts.
/// Uses Claude Sonnet pricing as default: $3/M input, $15/M output.
pub fn estimate_cost(input_tokens: u32, output_tokens: u32) -> f64 {
    let input_cost = (input_tokens as f64) * 3.0 / 1_000_000.0;
    let output_cost = (output_tokens as f64) * 15.0 / 1_000_000.0;
    ((input_cost + output_cost) * 1_000_000.0).round() / 1_000_000.0
}
