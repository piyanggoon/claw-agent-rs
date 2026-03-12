use std::sync::Arc;

use tokio::sync::broadcast;
use agent_sdk::{
    builder, AgentCapabilities, AgentConfig, AgentEvent, AgentEventEnvelope, AgentInput,
    AgentRunState, LocalFileSystem, ThreadId, Tool, ToolContext, ToolRegistry, ToolResult,
    ToolTier,
    primitive_tools::{ReadTool, WriteTool, EditTool, GlobTool, GrepTool, BashTool},
};
use serde_json::{json, Value};

use crate::context::ClawContext;
use crate::db::stores::{SqliteMessageStore, SqliteStateStore};
use crate::hooks::ClawHooks;
use crate::tools::register_all_tools;
use crate::soul::prompt::build_system_prompt;
use crate::agent::provider::create_provider;

// ── Context adapter ─────────────────────────────────────────────────────────
//
// SDK primitive tools implement `Tool<()>` but our registry is
// `ToolRegistry<ClawContext>`.  This wrapper bridges the gap by creating
// a dummy `ToolContext<()>` before delegating to the inner tool.

struct Adapt<T>(T);

impl<T: Tool<()> + Send + Sync + 'static> Tool<ClawContext> for Adapt<T> {
    type Name = T::Name;

    fn name(&self) -> Self::Name {
        self.0.name()
    }

    fn display_name(&self) -> &'static str {
        self.0.display_name()
    }

    fn description(&self) -> &'static str {
        self.0.description()
    }

    fn input_schema(&self) -> Value {
        self.0.input_schema()
    }

    fn tier(&self) -> ToolTier {
        self.0.tier()
    }

    async fn execute(
        &self,
        ctx: &ToolContext<ClawContext>,
        input: Value,
    ) -> anyhow::Result<ToolResult> {
        // Build a ToolContext<()> and forward event_tx + event_seq so that
        // SDK tools (including SubagentTool) can emit events to the parent stream.
        let mut unit_ctx = ToolContext::new(());
        if let (Some(tx), Some(seq)) = (ctx.event_tx(), ctx.event_seq()) {
            unit_ctx = unit_ctx.with_event_tx(tx, seq);
        }
        self.0.execute(&unit_ctx, input).await
    }
}

/// Result of an agent run with accumulated output data.
pub struct RunResult {
    pub accumulated_text: String,
    pub tool_calls: Vec<serde_json::Value>,
    pub total_turns: usize,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub duration_ms: u64,
}

/// Run an agent with the given prompt.
///
/// This:
/// 1. Builds system prompt from soul files
/// 2. Creates ToolRegistry with all custom tools
/// 3. Creates AgentLoop via builder
/// 4. Runs the agent and forwards events
/// 5. Returns accumulated text and metadata
pub async fn run_agent(
    ctx: ClawContext,
    thread_id: ThreadId,
    message: String,
    model: Option<String>,
    event_tx: broadcast::Sender<AgentEventEnvelope>,
) -> anyhow::Result<RunResult> {
    let config = ctx.config.clone();
    let model_name = model.unwrap_or_else(|| config.default_model.clone());

    // Build system prompt from AGENTS.md + soul files (using the active group)
    let agents_md_path = config.groups_dir
        .join(&ctx.group)
        .join("AGENTS.md");
    let system_prompt = build_system_prompt(
        &ctx.soul,
        &ctx.memory,
        &config.timezone,
        &agents_md_path,
    ).await;

    // Create provider
    let provider = create_provider(&model_name, &config);

    // Create tool registry with all custom + primitive tools
    let mut tools: ToolRegistry<ClawContext> = ToolRegistry::new();
    register_all_tools(&mut tools);

    // Register SDK primitive tools (Read, Write, Edit, Glob, Grep, Bash).
    // Wrapped with Adapt<> to bridge Tool<()> → Tool<ClawContext>.
    let fs = Arc::new(LocalFileSystem::new("/"));
    let capabilities = AgentCapabilities::full_access();
    tools
        .register(Adapt(ReadTool::new(Arc::clone(&fs), capabilities.clone())))
        .register(Adapt(WriteTool::new(Arc::clone(&fs), capabilities.clone())))
        .register(Adapt(EditTool::new(Arc::clone(&fs), capabilities.clone())))
        .register(Adapt(GlobTool::new(Arc::clone(&fs), capabilities.clone())))
        .register(Adapt(GrepTool::new(Arc::clone(&fs), capabilities.clone())))
        .register(Adapt(BashTool::new(Arc::clone(&fs), capabilities)));

    // Create hooks
    let hooks = ClawHooks::new(event_tx.clone());

    // Create stores
    let message_store = SqliteMessageStore::new(ctx.db.clone());
    let state_store = SqliteStateStore::new(ctx.db.clone());

    // Build agent config
    let agent_config = AgentConfig {
        system_prompt,
        model: model_name,
        max_turns: Some(100),
        streaming: true,
        ..Default::default()
    };

    // Build agent loop
    let agent = builder::<ClawContext>()
        .provider(provider)
        .tools(tools)
        .hooks(hooks)
        .message_store(message_store)
        .state_store(state_store)
        .config(agent_config)
        .build_with_stores();

    // Create tool context
    let tool_ctx = ToolContext::new(ctx);

    // Run the agent
    let (mut events, final_state) = agent.run(
        thread_id,
        AgentInput::Text(message),
        tool_ctx,
    );

    // Forward events to broadcast channel while accumulating text/metadata
    let mut accumulated_text = String::new();
    let mut tool_calls: Vec<serde_json::Value> = Vec::new();
    let mut result_meta: Option<(usize, u32, u32, u64)> = None;

    while let Some(envelope) = events.recv().await {
        // Accumulate data from events
        match &envelope.event {
            AgentEvent::TextDelta { delta, .. } => {
                accumulated_text.push_str(delta);
            }
            AgentEvent::ToolCallStart { id, name, input, .. } => {
                // contentSplitIndex must use UTF-16 code units (matching JavaScript's
                // string.length) — NOT Rust's byte count which differs for non-ASCII.
                let split_idx = accumulated_text.encode_utf16().count();
                tool_calls.push(json!({
                    "id": id,
                    "name": name,
                    "input": serde_json::to_string(input).unwrap_or_default(),
                    "status": "running",
                    "order": tool_calls.len(),
                    "contentSplitIndex": split_idx,
                }));
            }
            AgentEvent::ToolCallEnd { id, result, .. } => {
                for tc in &mut tool_calls {
                    if tc["id"].as_str() == Some(id) {
                        tc["output"] = json!(result.output);
                        tc["status"] = json!(if result.success { "done" } else { "error" });
                    }
                }
            }
            AgentEvent::Done { total_turns, total_usage, duration, .. } => {
                result_meta = Some((
                    *total_turns,
                    total_usage.input_tokens,
                    total_usage.output_tokens,
                    duration.as_millis() as u64,
                ));
            }
            _ => {}
        }

        let is_done = matches!(envelope.event, AgentEvent::Done { .. });
        let _ = event_tx.send(envelope);
        if is_done {
            break;
        }
    }

    // Wait for final state
    match final_state.await {
        Ok(AgentRunState::Done { .. }) => {
            tracing::info!("Agent completed successfully");
        }
        Ok(AgentRunState::Error(e)) => {
            tracing::error!("Agent error: {}", e);
        }
        Ok(state) => {
            tracing::info!("Agent finished with state: {:?}", state);
        }
        Err(e) => {
            tracing::error!("Agent state channel error: {}", e);
        }
    }

    let (total_turns, input_tokens, output_tokens, duration_ms) =
        result_meta.unwrap_or((0, 0, 0, 0));

    Ok(RunResult {
        accumulated_text,
        tool_calls,
        total_turns,
        input_tokens,
        output_tokens,
        duration_ms,
    })
}
