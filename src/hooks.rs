use agent_sdk::{AgentEvent, AgentEventEnvelope, AgentHooks, ToolDecision, ToolResult, ToolTier};
use async_trait::async_trait;
use tokio::sync::broadcast;

pub struct ClawHooks {
    event_tx: broadcast::Sender<AgentEventEnvelope>,
}

impl ClawHooks {
    pub fn new(event_tx: broadcast::Sender<AgentEventEnvelope>) -> Self {
        Self { event_tx }
    }
}

#[async_trait]
impl AgentHooks for ClawHooks {
    async fn pre_tool_use(
        &self,
        tool_name: &str,
        _input: &serde_json::Value,
        tier: ToolTier,
    ) -> ToolDecision {
        tracing::debug!(tool = tool_name, ?tier, "Pre tool use");
        // Allow all tools automatically (including Confirm tier for now)
        // In the future, we can route Confirm tier to user approval
        ToolDecision::Allow
    }

    async fn post_tool_use(&self, tool_name: &str, result: &ToolResult) {
        tracing::debug!(
            tool = tool_name,
            success = result.success,
            "Post tool use"
        );
    }

    async fn on_event(&self, event: &AgentEvent) {
        match event {
            AgentEvent::Done { total_turns, total_usage, .. } => {
                tracing::info!(
                    turns = total_turns,
                    input_tokens = total_usage.input_tokens,
                    output_tokens = total_usage.output_tokens,
                    "Agent completed"
                );
            }
            AgentEvent::Error { message, .. } => {
                tracing::error!(err = message, "Agent error event");
            }
            _ => {}
        }
    }

    async fn on_error(&self, error: &anyhow::Error) -> bool {
        tracing::error!(err = %error, "Agent error - attempting recovery");
        true // continue
    }

    async fn on_context_compact(&self, _messages: &[agent_sdk::llm::Message]) -> Option<String> {
        None // use default compaction
    }
}
