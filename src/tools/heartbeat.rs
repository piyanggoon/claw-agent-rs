use agent_sdk::{DynamicToolName, Tool, ToolContext, ToolResult, ToolTier};
use serde_json::{json, Value};
use std::future::Future;

use crate::context::ClawContext;

// ---------------------------------------------------------------------------
// HeartbeatReadTool
// ---------------------------------------------------------------------------

pub struct HeartbeatReadTool;

impl Tool<ClawContext> for HeartbeatReadTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("heartbeat_read")
    }

    fn display_name(&self) -> &'static str {
        "Read Heartbeat"
    }

    fn description(&self) -> &'static str {
        "Read the HEARTBEAT.md file. This file contains your current state: mood, energy, active focus, recent context. HEARTBEAT.md is your \"working memory\" that persists between conversations."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Observe
    }

    fn execute(
        &self,
        ctx: &ToolContext<ClawContext>,
        _input: Value,
    ) -> impl Future<Output = anyhow::Result<ToolResult>> + Send {
        let soul = ctx.app.soul.clone();

        async move {
            match soul.read("HEARTBEAT.md").await {
                Ok(content) => Ok(ToolResult::success(content)),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to read HEARTBEAT.md: {}",
                    e
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HeartbeatUpdateTool
// ---------------------------------------------------------------------------

pub struct HeartbeatUpdateTool;

impl Tool<ClawContext> for HeartbeatUpdateTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("heartbeat_update")
    }

    fn display_name(&self) -> &'static str {
        "Update Heartbeat"
    }

    fn description(&self) -> &'static str {
        "Update HEARTBEAT.md with your current state. Overwrites the entire file. This is your \"working memory\" — update it at the end of each conversation to capture your current state for the next session."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The full content to write to HEARTBEAT.md"
                }
            },
            "required": ["content"]
        })
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Observe
    }

    fn execute(
        &self,
        ctx: &ToolContext<ClawContext>,
        input: Value,
    ) -> impl Future<Output = anyhow::Result<ToolResult>> + Send {
        let soul = ctx.app.soul.clone();
        let content = input["content"].as_str().unwrap_or("").to_string();

        async move {
            if content.is_empty() {
                return Ok(ToolResult::error("content is required"));
            }

            match soul.write("HEARTBEAT.md", &content).await {
                Ok(()) => Ok(ToolResult::success(
                    "HEARTBEAT.md updated successfully.".to_string(),
                )),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to update HEARTBEAT.md: {}",
                    e
                ))),
            }
        }
    }
}
