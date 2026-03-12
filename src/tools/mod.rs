pub mod soul;
pub mod memory;
pub mod heartbeat;
pub mod tasks;
pub mod utility;
pub mod subagent;

use agent_sdk::ToolRegistry;
use crate::context::ClawContext;

pub fn register_all_tools(registry: &mut ToolRegistry<ClawContext>) {
    // Soul tools
    registry.register(soul::SoulReadTool);
    registry.register(soul::SoulUpdateTool);
    registry.register(soul::SoulUpdateSectionTool);
    registry.register(soul::SoulDeleteTool);
    // Memory tools
    registry.register(memory::MemorySaveTool);
    registry.register(memory::MemoryDailyLogTool);
    registry.register(memory::MemoryRecallTool);
    registry.register(memory::MemoryForgetTool);
    // Heartbeat tools
    registry.register(heartbeat::HeartbeatReadTool);
    registry.register(heartbeat::HeartbeatUpdateTool);
    // Task tools
    registry.register(tasks::ScheduleTaskTool);
    registry.register(tasks::ListTasksTool);
    registry.register(tasks::PauseTaskTool);
    registry.register(tasks::ResumeTaskTool);
    registry.register(tasks::CancelTaskTool);
    // Utility tools
    registry.register(utility::SendNotificationTool);
    registry.register(utility::SendChatMessageTool);
    registry.register(utility::AskUserTool);
    registry.register(utility::RunBackgroundTool);
    registry.register(utility::WebFetchTool);
    registry.register(utility::CodeExecuteTool);
    // Subagent tool
    registry.register(subagent::SubagentTool);
}
