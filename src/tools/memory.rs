use agent_sdk::{DynamicToolName, Tool, ToolContext, ToolResult, ToolTier};
use serde_json::{json, Value};
use std::future::Future;

use crate::context::ClawContext;

// ---------------------------------------------------------------------------
// MemorySaveTool
// ---------------------------------------------------------------------------

pub struct MemorySaveTool;

impl Tool<ClawContext> for MemorySaveTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("memory_save")
    }

    fn display_name(&self) -> &'static str {
        "Save to Memory"
    }

    fn description(&self) -> &'static str {
        "Save information to MEMORY.md. Memory is organized in sections (## headings). Use action \"append\" to add to a section or \"replace\" to overwrite the entire section."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "section": {
                    "type": "string",
                    "description": "The ## heading in MEMORY.md to write to (e.g. \"Facts\", \"Preferences\", \"Instructions\", \"Insights\", \"Context\")"
                },
                "content": {
                    "type": "string",
                    "description": "The content to save to the section"
                },
                "action": {
                    "type": "string",
                    "enum": ["append", "replace"],
                    "description": "Whether to append to the section (default) or replace it entirely"
                }
            },
            "required": ["section", "content"]
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
        let memory = ctx.app.memory.clone();
        let section = input["section"].as_str().unwrap_or("").to_string();
        let content = input["content"].as_str().unwrap_or("").to_string();
        let action = input["action"]
            .as_str()
            .unwrap_or("append")
            .to_string();

        async move {
            if section.is_empty() {
                return Ok(ToolResult::error("section is required"));
            }
            if content.is_empty() {
                return Ok(ToolResult::error("content is required"));
            }

            match memory.save(&section, &content, &action).await {
                Ok(()) => Ok(ToolResult::success(format!(
                    "Memory saved to section '{}' (action: {}).",
                    section, action
                ))),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to save memory to section '{}': {}",
                    section, e
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryDailyLogTool
// ---------------------------------------------------------------------------

pub struct MemoryDailyLogTool;

impl Tool<ClawContext> for MemoryDailyLogTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("memory_daily_log")
    }

    fn display_name(&self) -> &'static str {
        "Daily Log"
    }

    fn description(&self) -> &'static str {
        "Append an entry to today's daily log file at soul/memory/YYYY-MM-DD.md. Daily logs capture timestamped events, observations, and activities."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The log entry content"
                },
                "category": {
                    "type": "string",
                    "enum": ["event", "observation", "decision", "interaction", "reflection"],
                    "description": "Optional category for visual organization"
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
        let memory = ctx.app.memory.clone();
        let content = input["content"].as_str().unwrap_or("").to_string();
        let category = input["category"].as_str().map(|s| s.to_string());

        async move {
            if content.is_empty() {
                return Ok(ToolResult::error("content is required"));
            }

            match memory
                .daily_log(&content, category.as_deref())
                .await
            {
                Ok(()) => Ok(ToolResult::success("Daily log entry added.")),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to add daily log entry: {}",
                    e
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryRecallTool
// ---------------------------------------------------------------------------

pub struct MemoryRecallTool;

impl Tool<ClawContext> for MemoryRecallTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("memory_recall")
    }

    fn display_name(&self) -> &'static str {
        "Recall Memory"
    }

    fn description(&self) -> &'static str {
        "Search across MEMORY.md and daily log files for matching content. Performs a case-insensitive keyword search. Returns matching lines with their source file."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Optional search query to filter memories. If omitted, returns all memory content."
                },
                "days": {
                    "type": "number",
                    "description": "Number of days of daily logs to search (default: 7)"
                }
            }
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
        let memory = ctx.app.memory.clone();
        let query = input["query"].as_str().map(|s| s.to_string());
        let days = input["days"].as_u64().unwrap_or(7) as u32;

        async move {
            match memory.recall(query.as_deref(), days).await {
                Ok(content) => Ok(ToolResult::success(content)),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to recall memories: {}",
                    e
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryForgetTool
// ---------------------------------------------------------------------------

pub struct MemoryForgetTool;

impl Tool<ClawContext> for MemoryForgetTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("memory_forget")
    }

    fn display_name(&self) -> &'static str {
        "Forget Memory"
    }

    fn description(&self) -> &'static str {
        "Remove a specific entry from a section in MEMORY.md. Finds and removes lines matching the entry text within the specified section using substring matching (case-insensitive)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "section": {
                    "type": "string",
                    "description": "The ## heading in MEMORY.md to remove from (e.g. \"Facts\", \"Preferences\")"
                },
                "entry": {
                    "type": "string",
                    "description": "The text to match and remove (case-insensitive substring match)"
                }
            },
            "required": ["section", "entry"]
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
        let memory = ctx.app.memory.clone();
        let section = input["section"].as_str().unwrap_or("").to_string();
        let entry = input["entry"].as_str().unwrap_or("").to_string();

        async move {
            if section.is_empty() {
                return Ok(ToolResult::error("section is required"));
            }
            if entry.is_empty() {
                return Ok(ToolResult::error("entry is required"));
            }

            match memory.forget(&section, &entry).await {
                Ok(()) => Ok(ToolResult::success(format!(
                    "Matching entries removed from section '{}'.",
                    section
                ))),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to forget entry from section '{}': {}",
                    section, e
                ))),
            }
        }
    }
}
