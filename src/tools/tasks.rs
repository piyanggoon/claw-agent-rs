use std::future::Future;
use std::str::FromStr;

use agent_sdk::{DynamicToolName, Tool, ToolContext, ToolResult, ToolTier};
use chrono::{Duration, Utc};
use cron::Schedule;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::context::ClawContext;
use crate::db::tasks::{self, ScheduledTask};

/// Calculate the next run time based on schedule type and value.
///
/// - `"delay"`: now + milliseconds
/// - `"once"`: the schedule_value itself as an ISO datetime string
/// - `"interval"`: now + milliseconds
/// - `"cron"`: parse cron expression and find the next occurrence
fn calculate_next_run(schedule_type: &str, schedule_value: &str) -> Result<String, String> {
    match schedule_type {
        "delay" | "interval" => {
            let ms: i64 = schedule_value
                .parse()
                .map_err(|_| format!("Invalid milliseconds value: {}", schedule_value))?;
            let next = Utc::now() + Duration::milliseconds(ms);
            Ok(next.to_rfc3339())
        }
        "once" => {
            // The schedule_value is an ISO datetime string (local time without Z).
            // We store it as-is for the scheduler to interpret.
            // Try to parse to validate, but store the original.
            if schedule_value.is_empty() {
                return Err("schedule_value is required for 'once' type".to_string());
            }
            // If it already has timezone info, use as-is; otherwise append Z
            let value = if schedule_value.ends_with('Z') || schedule_value.contains('+') {
                schedule_value.to_string()
            } else {
                format!("{}Z", schedule_value)
            };
            // Validate it parses
            chrono::DateTime::parse_from_rfc3339(&value)
                .map_err(|e| format!("Invalid datetime '{}': {}", schedule_value, e))?;
            Ok(value)
        }
        "cron" => {
            let schedule = Schedule::from_str(schedule_value)
                .map_err(|e| format!("Invalid cron expression '{}': {}", schedule_value, e))?;
            let next = schedule
                .upcoming(Utc)
                .next()
                .ok_or_else(|| "No upcoming occurrences for cron expression".to_string())?;
            Ok(next.to_rfc3339())
        }
        _ => Err(format!("Unknown schedule_type: {}", schedule_type)),
    }
}

// ---------------------------------------------------------------------------
// ScheduleTaskTool
// ---------------------------------------------------------------------------

pub struct ScheduleTaskTool;

impl Tool<ClawContext> for ScheduleTaskTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("schedule_task")
    }

    fn display_name(&self) -> &'static str {
        "Schedule Task"
    }

    fn description(&self) -> &'static str {
        "Schedule a recurring or one-time task. The task will run as a full agent with access to all tools. Schedule types: \"cron\" (cron expression), \"interval\" (milliseconds between runs), \"once\" (ISO datetime), \"delay\" (milliseconds from now, runs once)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The prompt/instructions for the task agent"
                },
                "schedule_type": {
                    "type": "string",
                    "enum": ["cron", "interval", "once", "delay"],
                    "description": "The type of schedule"
                },
                "schedule_value": {
                    "type": "string",
                    "description": "The schedule value (cron expression, milliseconds, or ISO datetime)"
                },
                "context_mode": {
                    "type": "string",
                    "enum": ["group", "isolated"],
                    "description": "\"group\" runs with conversation context and memory; \"isolated\" runs fresh (default: \"group\")"
                }
            },
            "required": ["prompt", "schedule_type", "schedule_value"]
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
        let db = ctx.app.db.clone();
        let scheduler = ctx.app.scheduler.clone();
        let config = ctx.app.config.clone();
        let session_id = ctx.app.session_id.clone();

        let prompt = input["prompt"].as_str().unwrap_or("").to_string();
        let schedule_type = input["schedule_type"].as_str().unwrap_or("").to_string();
        let schedule_value = input["schedule_value"].as_str().unwrap_or("").to_string();
        let context_mode = input["context_mode"]
            .as_str()
            .unwrap_or("group")
            .to_string();

        async move {
            if prompt.is_empty() {
                return Ok(ToolResult::error("prompt is required"));
            }
            if schedule_type.is_empty() {
                return Ok(ToolResult::error("schedule_type is required"));
            }
            if schedule_value.is_empty() {
                return Ok(ToolResult::error("schedule_value is required"));
            }

            let next_run = match calculate_next_run(&schedule_type, &schedule_value) {
                Ok(nr) => nr,
                Err(e) => return Ok(ToolResult::error(e)),
            };

            let task_id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();

            let task = ScheduledTask {
                id: task_id.clone(),
                group_folder: config.main_group.clone(),
                prompt,
                schedule_type,
                schedule_value,
                context_mode,
                context_session: session_id,
                next_run: Some(next_run),
                last_run: None,
                last_result: None,
                status: "active".to_string(),
                created_at: now,
            };

            {
                let conn = db.lock().await;
                if let Err(e) = tasks::create_task(&conn, &task) {
                    return Ok(ToolResult::error(format!(
                        "Failed to create task: {}",
                        e
                    )));
                }
            }

            // Notify the scheduler that a new task has been added
            scheduler.notify_new_task();

            Ok(ToolResult::success(json!({
                "task_id": task_id,
                "status": "active",
                "next_run": task.next_run,
                "message": "Task scheduled successfully."
            }).to_string()))
        }
    }
}

// ---------------------------------------------------------------------------
// ListTasksTool
// ---------------------------------------------------------------------------

pub struct ListTasksTool;

impl Tool<ClawContext> for ListTasksTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("list_tasks")
    }

    fn display_name(&self) -> &'static str {
        "List Tasks"
    }

    fn description(&self) -> &'static str {
        "List all scheduled tasks."
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
        let db = ctx.app.db.clone();

        async move {
            let conn = db.lock().await;
            match tasks::get_all_tasks(&conn) {
                Ok(task_list) => {
                    let output = serde_json::to_string_pretty(&task_list)
                        .unwrap_or_else(|_| "[]".to_string());
                    Ok(ToolResult::success(output))
                }
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to list tasks: {}",
                    e
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PauseTaskTool
// ---------------------------------------------------------------------------

pub struct PauseTaskTool;

impl Tool<ClawContext> for PauseTaskTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("pause_task")
    }

    fn display_name(&self) -> &'static str {
        "Pause Task"
    }

    fn description(&self) -> &'static str {
        "Pause a scheduled task. It will not run until resumed."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The ID of the task to pause"
                }
            },
            "required": ["task_id"]
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
        let db = ctx.app.db.clone();
        let task_id = input["task_id"].as_str().unwrap_or("").to_string();

        async move {
            if task_id.is_empty() {
                return Ok(ToolResult::error("task_id is required"));
            }

            let conn = db.lock().await;
            match tasks::update_task_status(&conn, &task_id, "paused") {
                Ok(()) => Ok(ToolResult::success(format!(
                    "Task '{}' paused successfully.",
                    task_id
                ))),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to pause task '{}': {}",
                    task_id, e
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ResumeTaskTool
// ---------------------------------------------------------------------------

pub struct ResumeTaskTool;

impl Tool<ClawContext> for ResumeTaskTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("resume_task")
    }

    fn display_name(&self) -> &'static str {
        "Resume Task"
    }

    fn description(&self) -> &'static str {
        "Resume a paused task."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The ID of the task to resume"
                }
            },
            "required": ["task_id"]
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
        let db = ctx.app.db.clone();
        let task_id = input["task_id"].as_str().unwrap_or("").to_string();

        async move {
            if task_id.is_empty() {
                return Ok(ToolResult::error("task_id is required"));
            }

            let conn = db.lock().await;
            match tasks::update_task_status(&conn, &task_id, "active") {
                Ok(()) => Ok(ToolResult::success(format!(
                    "Task '{}' resumed successfully.",
                    task_id
                ))),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to resume task '{}': {}",
                    task_id, e
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CancelTaskTool
// ---------------------------------------------------------------------------

pub struct CancelTaskTool;

impl Tool<ClawContext> for CancelTaskTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("cancel_task")
    }

    fn display_name(&self) -> &'static str {
        "Cancel Task"
    }

    fn description(&self) -> &'static str {
        "Cancel and delete a scheduled task."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The ID of the task to cancel and delete"
                }
            },
            "required": ["task_id"]
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
        let db = ctx.app.db.clone();
        let task_id = input["task_id"].as_str().unwrap_or("").to_string();

        async move {
            if task_id.is_empty() {
                return Ok(ToolResult::error("task_id is required"));
            }

            let conn = db.lock().await;
            match tasks::delete_task(&conn, &task_id) {
                Ok(()) => Ok(ToolResult::success(format!(
                    "Task '{}' cancelled and deleted.",
                    task_id
                ))),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to cancel task '{}': {}",
                    task_id, e
                ))),
            }
        }
    }
}
