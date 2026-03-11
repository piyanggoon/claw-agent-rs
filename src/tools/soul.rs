use agent_sdk::{DynamicToolName, Tool, ToolContext, ToolResult, ToolTier};
use serde_json::{json, Value};
use std::future::Future;

use crate::context::ClawContext;

// ---------------------------------------------------------------------------
// SoulReadTool
// ---------------------------------------------------------------------------

pub struct SoulReadTool;

impl Tool<ClawContext> for SoulReadTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("soul_read")
    }

    fn display_name(&self) -> &'static str {
        "Read Soul File"
    }

    fn description(&self) -> &'static str {
        "Read a soul file from disk. Soul files live in the soul/ directory and contain identity, personality, memories, and configuration."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "The soul file to read (e.g. \"SOUL.md\", \"MEMORY.md\", \"memory/2026-03-03.md\")"
                }
            },
            "required": ["filename"]
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
        let filename = input["filename"].as_str().unwrap_or("").to_string();

        async move {
            if filename.is_empty() {
                return Ok(ToolResult::error("filename is required"));
            }

            match soul.read(&filename).await {
                Ok(content) => Ok(ToolResult::success(content)),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to read soul file '{}': {}",
                    filename, e
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SoulUpdateTool
// ---------------------------------------------------------------------------

pub struct SoulUpdateTool;

impl Tool<ClawContext> for SoulUpdateTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("soul_update")
    }

    fn display_name(&self) -> &'static str {
        "Update Soul File"
    }

    fn description(&self) -> &'static str {
        "Write entire content to a soul file. Overwrites the file completely. Use soul_update_section for updating a single section instead."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "The soul file to write (e.g. \"SOUL.md\", \"IDENTITY.md\")"
                },
                "content": {
                    "type": "string",
                    "description": "The full content to write to the file"
                }
            },
            "required": ["filename", "content"]
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
        let filename = input["filename"].as_str().unwrap_or("").to_string();
        let content = input["content"].as_str().unwrap_or("").to_string();

        async move {
            if filename.is_empty() {
                return Ok(ToolResult::error("filename is required"));
            }
            if content.is_empty() {
                return Ok(ToolResult::error("content is required"));
            }

            match soul.write(&filename, &content).await {
                Ok(()) => Ok(ToolResult::success(format!(
                    "Soul file '{}' updated successfully.",
                    filename
                ))),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to write soul file '{}': {}",
                    filename, e
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SoulUpdateSectionTool
// ---------------------------------------------------------------------------

pub struct SoulUpdateSectionTool;

impl Tool<ClawContext> for SoulUpdateSectionTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("soul_update_section")
    }

    fn display_name(&self) -> &'static str {
        "Update Soul File Section"
    }

    fn description(&self) -> &'static str {
        "Update a specific markdown section (## heading) within a soul file. If the section exists, its content is replaced. If not, the section is appended at the end of the file."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "The soul file to update (e.g. \"MEMORY.md\", \"HEARTBEAT.md\")"
                },
                "heading": {
                    "type": "string",
                    "description": "The ## heading name of the section to update (e.g. \"Facts\", \"Current Mood\")"
                },
                "content": {
                    "type": "string",
                    "description": "The new content for the section"
                }
            },
            "required": ["filename", "heading", "content"]
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
        let filename = input["filename"].as_str().unwrap_or("").to_string();
        let heading = input["heading"].as_str().unwrap_or("").to_string();
        let content = input["content"].as_str().unwrap_or("").to_string();

        async move {
            if filename.is_empty() {
                return Ok(ToolResult::error("filename is required"));
            }
            if heading.is_empty() {
                return Ok(ToolResult::error("heading is required"));
            }

            match soul.update_section(&filename, &heading, &content).await {
                Ok(()) => Ok(ToolResult::success(format!(
                    "Section '## {}' in '{}' updated successfully.",
                    heading, filename
                ))),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to update section '## {}' in '{}': {}",
                    heading, filename, e
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SoulDeleteTool
// ---------------------------------------------------------------------------

pub struct SoulDeleteTool;

impl Tool<ClawContext> for SoulDeleteTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("soul_delete")
    }

    fn display_name(&self) -> &'static str {
        "Delete Soul File"
    }

    fn description(&self) -> &'static str {
        "Delete a soul file. Only BOOTSTRAP.md can be deleted (safety constraint). BOOTSTRAP.md contains one-time setup instructions that should be deleted after the agent has processed them."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "The soul file to delete (only \"BOOTSTRAP.md\" is allowed)"
                }
            },
            "required": ["filename"]
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
        let filename = input["filename"].as_str().unwrap_or("").to_string();

        async move {
            if filename.is_empty() {
                return Ok(ToolResult::error("filename is required"));
            }

            if filename != "BOOTSTRAP.md" {
                return Ok(ToolResult::error(format!(
                    "Only BOOTSTRAP.md can be deleted (attempted: {})",
                    filename
                )));
            }

            match soul.delete(&filename).await {
                Ok(()) => Ok(ToolResult::success(
                    "BOOTSTRAP.md deleted successfully.".to_string(),
                )),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to delete '{}': {}",
                    filename, e
                ))),
            }
        }
    }
}
