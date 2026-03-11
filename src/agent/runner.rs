use tokio::sync::broadcast;
use agent_sdk::{
    builder, AgentConfig, AgentEvent, AgentEventEnvelope, AgentInput, AgentRunState,
    ThreadId, ToolContext, ToolRegistry,
};

use crate::context::ClawContext;
use crate::db::stores::{SqliteMessageStore, SqliteStateStore};
use crate::hooks::ClawHooks;
use crate::tools::register_all_tools;
use crate::soul::prompt::build_system_prompt;
use crate::agent::provider::create_provider;

/// Run an agent with the given prompt. Returns the run_id.
/// This spawns a tokio task that:
/// 1. Builds system prompt from soul files
/// 2. Creates ToolRegistry with all custom tools
/// 3. Creates AgentLoop via builder
/// 4. Runs the agent and forwards events
pub async fn run_agent(
    ctx: ClawContext,
    thread_id: ThreadId,
    message: String,
    model: Option<String>,
    event_tx: broadcast::Sender<AgentEventEnvelope>,
) -> anyhow::Result<()> {
    let config = ctx.config.clone();
    let model_name = model.unwrap_or_else(|| config.default_model.clone());

    // Build system prompt from AGENTS.md + soul files
    let agents_md_path = config.groups_dir
        .join(&config.main_group)
        .join("AGENTS.md");
    let system_prompt = build_system_prompt(
        &ctx.soul,
        &ctx.memory,
        &config.timezone,
        &agents_md_path,
    ).await;

    // Create provider
    let provider = create_provider(&model_name, &config);

    // Create tool registry with all custom tools
    let mut tools: ToolRegistry<ClawContext> = ToolRegistry::new();
    register_all_tools(&mut tools);

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

    // Forward events to broadcast channel
    while let Some(envelope) = events.recv().await {
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

    Ok(())
}
