//! Claw-native subagent tool.
//!
//! Spawns an isolated child agent with SDK primitive tools (Read, Write, Edit,
//! Glob, Grep, Bash) that runs to completion and returns its final response.
//! Progress events (`SubagentProgress`) are forwarded to the parent's SSE
//! stream so the frontend can display live tool-call updates.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_sdk::{
    AgentCapabilities, AgentConfig, AgentEvent, AgentEventEnvelope, AgentInput,
    DefaultHooks, DynamicToolName, InMemoryStore, LocalFileSystem, SequenceCounter,
    ThreadId, Tool, ToolContext, ToolRegistry, ToolResult, ToolTier,
    primitive_tools::{ReadTool, WriteTool, EditTool, GlobTool, GrepTool, BashTool},
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::context::ClawContext;
use crate::agent::provider::create_provider;

// ── Subagent tool ────────────────────────────────────────────────────────────

pub struct SubagentTool;

impl Tool<ClawContext> for SubagentTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("run_subagent")
    }

    fn display_name(&self) -> &'static str {
        "Run Subagent"
    }

    fn description(&self) -> &'static str {
        "Spawn an isolated subagent that runs to completion and returns only its \
         final response. The subagent has access to file-system tools (Read, Write, \
         Edit, Glob, Grep, Bash) but NOT soul/memory/task tools. Use for delegating \
         complex subtasks like code analysis, refactoring, or multi-file edits. \
         Progress is streamed to the UI in real-time."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The task or instructions for the subagent"
                },
                "system_prompt": {
                    "type": "string",
                    "description": "Optional system prompt override for the subagent. If omitted, a sensible default is used."
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Maximum turns the subagent can take (default: 30)"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default: 300000 = 5 minutes)"
                },
                "model": {
                    "type": "string",
                    "description": "Override model for the subagent (default: same as parent)"
                }
            },
            "required": ["task"]
        })
    }

    fn tier(&self) -> ToolTier {
        // Subagent spawning requires confirmation (same as SDK)
        ToolTier::Confirm
    }

    fn execute(
        &self,
        ctx: &ToolContext<ClawContext>,
        input: Value,
    ) -> impl Future<Output = anyhow::Result<ToolResult>> + Send {
        let claw_ctx = ctx.app.clone();
        let parent_tx = ctx.event_tx();
        let parent_seq = ctx.event_seq();

        let task = input["task"].as_str().unwrap_or("").to_string();
        let system_prompt = input["system_prompt"]
            .as_str()
            .map(|s| s.to_string());
        let max_turns = input["max_turns"].as_u64().unwrap_or(30) as usize;
        let timeout_ms = input["timeout_ms"].as_u64().unwrap_or(300_000);
        let model = input["model"].as_str().map(|s| s.to_string());

        async move {
            if task.is_empty() {
                return Ok(ToolResult::error("task is required"));
            }

            let result = run_subagent(
                &claw_ctx,
                &task,
                system_prompt.as_deref(),
                max_turns,
                timeout_ms,
                model.as_deref(),
                parent_tx,
                parent_seq,
            )
            .await;

            match result {
                Ok(sub_result) => {
                    let output = if sub_result.success {
                        sub_result.final_response
                    } else {
                        format!("[Subagent failed] {}", sub_result.final_response)
                    };
                    let data = json!({
                        "total_turns": sub_result.total_turns,
                        "tool_count": sub_result.tool_count,
                        "duration_ms": sub_result.duration_ms,
                        "input_tokens": sub_result.input_tokens,
                        "output_tokens": sub_result.output_tokens,
                        "success": sub_result.success,
                    });

                    Ok(ToolResult {
                        success: sub_result.success,
                        output,
                        data: Some(data),
                        documents: Vec::new(),
                        duration_ms: Some(sub_result.duration_ms),
                    })
                }
                Err(e) => Ok(ToolResult::error(format!("Subagent error: {e}"))),
            }
        }
    }
}

// ── Subagent result ──────────────────────────────────────────────────────────

struct SubagentRunResult {
    final_response: String,
    total_turns: usize,
    tool_count: u32,
    input_tokens: u32,
    output_tokens: u32,
    duration_ms: u64,
    success: bool,
}

// ── Subagent execution ───────────────────────────────────────────────────────

const DEFAULT_SUBAGENT_PROMPT: &str = "\
You are a focused task executor. Complete the given task efficiently using \
the available file-system tools. Be thorough but concise in your final response. \
Summarize what you did and any important findings.";

/// Build a `ToolRegistry<()>` with SDK primitive tools.
fn build_primitive_registry() -> ToolRegistry<()> {
    let fs = Arc::new(LocalFileSystem::new("/"));
    let caps = AgentCapabilities::full_access();
    let mut reg = ToolRegistry::new();
    reg.register(ReadTool::new(Arc::clone(&fs), caps.clone()))
        .register(WriteTool::new(Arc::clone(&fs), caps.clone()))
        .register(EditTool::new(Arc::clone(&fs), caps.clone()))
        .register(GlobTool::new(Arc::clone(&fs), caps.clone()))
        .register(GrepTool::new(Arc::clone(&fs), caps.clone()))
        .register(BashTool::new(Arc::clone(&fs), caps));
    reg
}

/// Run an isolated subagent with SDK primitive tools.
///
/// If `parent_tx` + `parent_seq` are provided, `SubagentProgress` events are
/// emitted so the frontend can show live tool-call updates.
#[allow(clippy::too_many_arguments)]
async fn run_subagent(
    claw_ctx: &ClawContext,
    task: &str,
    system_prompt: Option<&str>,
    max_turns: usize,
    timeout_ms: u64,
    model: Option<&str>,
    parent_tx: Option<mpsc::Sender<AgentEventEnvelope>>,
    parent_seq: Option<SequenceCounter>,
) -> anyhow::Result<SubagentRunResult> {
    use agent_sdk::AgentLoop;

    let start = Instant::now();
    let config = &claw_ctx.config;

    let model_name = model.unwrap_or(&config.default_model);
    let provider = create_provider(model_name, config);
    let tools = build_primitive_registry();

    let prompt = system_prompt.unwrap_or(DEFAULT_SUBAGENT_PROMPT);

    let agent_config = AgentConfig {
        system_prompt: prompt.to_string(),
        model: model_name.to_string(),
        max_turns: Some(max_turns),
        streaming: true,
        ..Default::default()
    };

    let agent = AgentLoop::new(
        provider,
        tools,
        DefaultHooks,
        InMemoryStore::new(),
        InMemoryStore::new(),
        agent_config,
    );

    let thread_id = ThreadId::new();
    let tool_ctx = ToolContext::new(());

    let (mut rx, _final_state) = agent.run(
        thread_id,
        AgentInput::Text(task.to_string()),
        tool_ctx,
    );

    // Generate a unique subagent ID for progress events
    let subagent_id = format!(
        "subagent_{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );

    let mut final_response = String::new();
    let mut total_turns = 0usize;
    let mut tool_count = 0u32;
    let mut input_tokens = 0u32;
    let mut output_tokens = 0u32;
    let mut success = true;
    let timeout = Duration::from_millis(timeout_ms);

    loop {
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            final_response = "Subagent timed out".to_string();
            success = false;
            break;
        }

        let recv = tokio::time::timeout(remaining, rx.recv()).await;

        match recv {
            Ok(Some(envelope)) => match &envelope.event {
                AgentEvent::Text { text, .. } => {
                    final_response.push_str(text);
                }
                AgentEvent::TextDelta { delta, .. } => {
                    final_response.push_str(delta);
                }
                AgentEvent::ToolCallStart { name, input, .. } => {
                    tool_count += 1;
                    let context = extract_tool_context(name, input);

                    // Emit SubagentProgress → parent SSE
                    if let (Some(tx), Some(seq)) = (&parent_tx, &parent_seq) {
                        let event = AgentEvent::SubagentProgress {
                            subagent_id: subagent_id.clone(),
                            subagent_name: "subagent".to_string(),
                            tool_name: name.clone(),
                            tool_context: context,
                            completed: false,
                            success: false,
                            tool_count,
                            total_tokens: u64::from(input_tokens) + u64::from(output_tokens),
                        };
                        let _ = tx.send(AgentEventEnvelope::wrap(event, seq)).await;
                    }
                }
                AgentEvent::ToolCallEnd { name, result, .. } => {
                    let context = summarize_result(name, result);

                    // Emit SubagentProgress (completed) → parent SSE
                    if let (Some(tx), Some(seq)) = (&parent_tx, &parent_seq) {
                        let event = AgentEvent::SubagentProgress {
                            subagent_id: subagent_id.clone(),
                            subagent_name: "subagent".to_string(),
                            tool_name: name.clone(),
                            tool_context: context,
                            completed: true,
                            success: result.success,
                            tool_count,
                            total_tokens: u64::from(input_tokens) + u64::from(output_tokens),
                        };
                        let _ = tx.send(AgentEventEnvelope::wrap(event, seq)).await;
                    }
                }
                AgentEvent::TurnComplete { turn, usage, .. } => {
                    total_turns = *turn;
                    input_tokens = input_tokens.saturating_add(usage.input_tokens);
                    output_tokens = output_tokens.saturating_add(usage.output_tokens);
                }
                AgentEvent::Done { total_turns: t, .. } => {
                    total_turns = *t;
                    break;
                }
                AgentEvent::Error { message, .. } => {
                    final_response = message.clone();
                    success = false;
                    break;
                }
                _ => {}
            },
            Ok(None) => break,
            Err(_) => {
                final_response = "Subagent timed out".to_string();
                success = false;
                break;
            }
        }
    }

    Ok(SubagentRunResult {
        final_response,
        total_turns,
        tool_count,
        input_tokens,
        output_tokens,
        duration_ms: start.elapsed().as_millis() as u64,
        success,
    })
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract a brief context string from tool input for display.
fn extract_tool_context(name: &str, input: &Value) -> String {
    match name {
        "Read" | "read" => input
            .get("path")
            .or_else(|| input.get("file_path"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        "Write" | "write" | "Edit" | "edit" => input
            .get("path")
            .or_else(|| input.get("file_path"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        "Bash" | "bash" => {
            let cmd = input.get("command").and_then(Value::as_str).unwrap_or("");
            if cmd.len() > 60 {
                format!("{}…", &cmd[..57])
            } else {
                cmd.to_string()
            }
        }
        "Glob" | "glob" => input
            .get("pattern")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        "Grep" | "grep" => input
            .get("pattern")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

/// Summarize a tool result for progress display.
fn summarize_result(name: &str, result: &ToolResult) -> String {
    if !result.success {
        let first = result.output.lines().next().unwrap_or("Error");
        return if first.len() > 60 {
            format!("{}…", &first[..57])
        } else {
            first.to_string()
        };
    }

    match name {
        "Read" | "read" => format!("{} lines", result.output.lines().count()),
        "Write" | "write" => "wrote file".to_string(),
        "Edit" | "edit" => "edited".to_string(),
        "Bash" | "bash" => {
            let lines: Vec<&str> = result.output.lines().collect();
            match lines.len() {
                0 => "done".to_string(),
                1 => {
                    let l = lines[0];
                    if l.len() > 60 { format!("{}…", &l[..57]) } else { l.to_string() }
                }
                n => format!("{n} lines"),
            }
        }
        "Glob" | "glob" => format!("{} files", result.output.lines().count()),
        "Grep" | "grep" => format!("{} matches", result.output.lines().count()),
        _ => {
            let n = result.output.lines().count();
            if n == 0 { "done".to_string() } else { format!("{n} lines") }
        }
    }
}
