use std::future::Future;
use std::time::Duration;

use agent_sdk::{DynamicToolName, Tool, ToolContext, ToolResult, ToolTier};
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::context::{ChatMessageEvent, ClawContext, NotificationEvent};
use crate::db::notifications as notifications_db;
use crate::db::tasks::{self, ScheduledTask};

// ---------------------------------------------------------------------------
// SendNotificationTool
// ---------------------------------------------------------------------------

pub struct SendNotificationTool;

impl Tool<ClawContext> for SendNotificationTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("send_notification")
    }

    fn display_name(&self) -> &'static str {
        "Send Notification"
    }

    fn description(&self) -> &'static str {
        "Send a notification to the web UI. Persisted — visible even if the user opens the UI later."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Notification title"
                },
                "message": {
                    "type": "string",
                    "description": "Notification message body"
                },
                "level": {
                    "type": "string",
                    "enum": ["info", "success", "warning", "error"],
                    "description": "Notification severity level (default: \"info\")"
                }
            },
            "required": ["title", "message"]
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
        let notification_tx = ctx.app.notification_tx.clone();

        let title = input["title"].as_str().unwrap_or("").to_string();
        let message = input["message"].as_str().unwrap_or("").to_string();
        let level = input["level"]
            .as_str()
            .unwrap_or("info")
            .to_string();

        async move {
            if title.is_empty() {
                return Ok(ToolResult::error("title is required"));
            }
            if message.is_empty() {
                return Ok(ToolResult::error("message is required"));
            }

            let id = Uuid::new_v4().to_string();

            // Persist to database
            {
                let conn = db.lock().await;
                if let Err(e) = notifications_db::create_notification(
                    &conn, &id, &title, &message, &level, "agent", None,
                ) {
                    return Ok(ToolResult::error(format!(
                        "Failed to create notification: {}",
                        e
                    )));
                }
            }

            // Broadcast to connected clients
            let event = NotificationEvent {
                id: id.clone(),
                title: title.clone(),
                message: message.clone(),
                level: level.clone(),
            };
            // Ignore send error (no receivers is fine)
            let _ = notification_tx.send(event);

            Ok(ToolResult::success(format!(
                "Notification sent: [{}] {} — {}",
                level, title, message
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// SendChatMessageTool
// ---------------------------------------------------------------------------

pub struct SendChatMessageTool;

impl Tool<ClawContext> for SendChatMessageTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("send_chat_message")
    }

    fn display_name(&self) -> &'static str {
        "Send Chat Message"
    }

    fn description(&self) -> &'static str {
        "Send a message directly to the web chat UI as a conversation message. Supports Markdown. Use when a background/scheduled task needs to report results back to the chat."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The message content (Markdown supported)"
                },
                "web_session_id": {
                    "type": "string",
                    "description": "Target web session ID. If not specified, the message goes to the most recent session."
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
        let chat_tx = ctx.app.chat_tx.clone();
        let fallback_session_id = ctx.app.session_id.clone();

        let content = input["content"].as_str().unwrap_or("").to_string();
        let web_session_id = input["web_session_id"]
            .as_str()
            .map(|s| s.to_string());

        async move {
            if content.is_empty() {
                return Ok(ToolResult::error("content is required"));
            }

            let session_id = web_session_id
                .or(fallback_session_id)
                .unwrap_or_else(|| "default".to_string());

            let event = ChatMessageEvent {
                session_id,
                content: content.clone(),
            };
            // Ignore send error (no receivers is fine)
            let _ = chat_tx.send(event);

            Ok(ToolResult::success("Chat message sent."))
        }
    }
}

// ---------------------------------------------------------------------------
// AskUserTool
// ---------------------------------------------------------------------------

pub struct AskUserTool;

impl Tool<ClawContext> for AskUserTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("ask_user")
    }

    fn display_name(&self) -> &'static str {
        "Ask User"
    }

    fn description(&self) -> &'static str {
        "Ask the user a question and wait for their response. The user sees the question in the chat UI."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of choices for the user to pick from"
                }
            },
            "required": ["question"]
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
        let chat_tx = ctx.app.chat_tx.clone();
        let custom_event_tx = ctx.app.custom_event_tx.clone();
        let pending_questions = ctx.app.pending_questions.clone();
        let session_id = ctx.app.session_id.clone();

        let question = input["question"].as_str().unwrap_or("").to_string();
        let options: Option<Vec<String>> = input["options"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            });

        async move {
            if question.is_empty() {
                return Ok(ToolResult::error("question is required"));
            }

            let question_id = Uuid::new_v4().to_string();

            // Build the question payload
            let payload = json!({
                "type": "ask_user",
                "question_id": question_id,
                "question": question,
                "options": options,
            });

            // Send via custom_event_tx if available (injects directly into SSE stream),
            // otherwise fall back to chat_tx broadcast
            if let Some(custom_tx) = &custom_event_tx {
                let _ = custom_tx.send(payload);
            } else {
                let target_session = session_id.unwrap_or_else(|| "default".to_string());
                let event = ChatMessageEvent {
                    session_id: target_session,
                    content: payload.to_string(),
                };
                let _ = chat_tx.send(event);
            }

            // Create a oneshot channel and register it for the web handler to resolve
            let (tx, rx) = oneshot::channel::<String>();
            pending_questions.insert(question_id.clone(), tx);

            // Wait for the user's response with a 5 minute timeout
            match tokio::time::timeout(Duration::from_secs(300), rx).await {
                Ok(Ok(answer)) => Ok(ToolResult::success(answer)),
                Ok(Err(_)) => {
                    // oneshot sender was dropped without sending
                    pending_questions.remove(&question_id);
                    Ok(ToolResult::error(
                        "Question was cancelled (no response received).",
                    ))
                }
                Err(_) => {
                    // Timeout elapsed
                    pending_questions.remove(&question_id);
                    Ok(ToolResult::error(
                        "Timed out waiting for user response (5 minutes).",
                    ))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RunBackgroundTool
// ---------------------------------------------------------------------------

pub struct RunBackgroundTool;

impl Tool<ClawContext> for RunBackgroundTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("run_background")
    }

    fn display_name(&self) -> &'static str {
        "Run Background Task"
    }

    fn description(&self) -> &'static str {
        "Run a task in the background and return to the chat immediately. The background agent runs isolated with access to all tools. It MUST use send_notification to report results."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The prompt/instructions for the background task"
                }
            },
            "required": ["prompt"]
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

        async move {
            if prompt.is_empty() {
                return Ok(ToolResult::error("prompt is required"));
            }

            let task_id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();

            // Create an immediate task (delay = 0ms)
            let next_run = Utc::now().to_rfc3339();

            let task = ScheduledTask {
                id: task_id.clone(),
                group_folder: config.main_group.clone(),
                prompt,
                schedule_type: "delay".to_string(),
                schedule_value: "0".to_string(),
                context_mode: "isolated".to_string(),
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
                        "Failed to create background task: {}",
                        e
                    )));
                }
            }

            // Notify the scheduler to pick up the new task immediately
            scheduler.notify_new_task();

            Ok(ToolResult::success(json!({
                "task_id": task_id,
                "message": "Background task created. It will run immediately and report results via send_notification."
            }).to_string()))
        }
    }
}

// ---------------------------------------------------------------------------
// WebFetchTool
// ---------------------------------------------------------------------------

pub struct WebFetchTool;

impl Tool<ClawContext> for WebFetchTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("web_fetch")
    }

    fn display_name(&self) -> &'static str {
        "Fetch Web Content"
    }

    fn description(&self) -> &'static str {
        "Fetch content from a URL and return it as clean text. Converts HTML to readable text. Useful for reading articles, docs, and web pages."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from"
                },
                "selector": {
                    "type": "string",
                    "description": "Optional CSS selector to extract specific content (not yet implemented, reserved for future use)"
                },
                "max_length": {
                    "type": "number",
                    "description": "Maximum character length of returned content (default: 50000)"
                }
            },
            "required": ["url"]
        })
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Observe
    }

    fn execute(
        &self,
        _ctx: &ToolContext<ClawContext>,
        input: Value,
    ) -> impl Future<Output = anyhow::Result<ToolResult>> + Send {
        let url = input["url"].as_str().unwrap_or("").to_string();
        let max_length = input["max_length"].as_u64().unwrap_or(50000) as usize;

        async move {
            if url.is_empty() {
                return Ok(ToolResult::error("url is required"));
            }

            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .user_agent("Mozilla/5.0 (compatible; ClawAgent/1.0)")
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

            let response = match client.get(&url).send().await {
                Ok(resp) => resp,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Failed to fetch URL '{}': {}",
                        url, e
                    )));
                }
            };

            let status = response.status();
            if !status.is_success() {
                return Ok(ToolResult::error(format!(
                    "HTTP {} for URL '{}'",
                    status, url
                )));
            }

            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            let body = match response.text().await {
                Ok(text) => text,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Failed to read response body: {}",
                        e
                    )));
                }
            };

            // Convert HTML to plain text if the content is HTML
            let text = if content_type.contains("text/html") || body.trim_start().starts_with('<')
            {
                html2text::from_read(body.as_bytes(), 80)
                    .unwrap_or_else(|_| body.clone())
            } else {
                body
            };

            // Truncate to max_length
            let output = if text.len() > max_length {
                let truncated = &text[..max_length];
                format!(
                    "{}\n\n[Content truncated at {} characters]",
                    truncated, max_length
                )
            } else {
                text
            };

            Ok(ToolResult::success(output))
        }
    }
}

// ---------------------------------------------------------------------------
// CodeExecuteTool
// ---------------------------------------------------------------------------

pub struct CodeExecuteTool;

impl Tool<ClawContext> for CodeExecuteTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName {
        DynamicToolName::new("code_execute")
    }

    fn display_name(&self) -> &'static str {
        "Execute Code"
    }

    fn description(&self) -> &'static str {
        "Execute code and return the output. Supported languages: javascript (Node.js), python (Python 3), bash (Shell). Returns both stdout and stderr."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "language": {
                    "type": "string",
                    "enum": ["javascript", "python", "bash"],
                    "description": "The programming language to execute"
                },
                "code": {
                    "type": "string",
                    "description": "The code to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Execution timeout in milliseconds (default: 10000, max: 30000)"
                }
            },
            "required": ["language", "code"]
        })
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Confirm
    }

    fn execute(
        &self,
        _ctx: &ToolContext<ClawContext>,
        input: Value,
    ) -> impl Future<Output = anyhow::Result<ToolResult>> + Send {
        let language = input["language"].as_str().unwrap_or("").to_string();
        let code = input["code"].as_str().unwrap_or("").to_string();
        let timeout_ms = input["timeout"]
            .as_u64()
            .unwrap_or(10000)
            .min(30000);

        async move {
            if language.is_empty() {
                return Ok(ToolResult::error("language is required"));
            }
            if code.is_empty() {
                return Ok(ToolResult::error("code is required"));
            }

            let (cmd, args): (&str, Vec<&str>) = match language.as_str() {
                "javascript" => ("node", vec!["-e", &code]),
                "python" => ("python3", vec!["-c", &code]),
                "bash" => ("bash", vec!["-c", &code]),
                _ => {
                    return Ok(ToolResult::error(format!(
                        "Unsupported language: {}. Supported: javascript, python, bash",
                        language
                    )));
                }
            };

            let child = tokio::process::Command::new(cmd)
                .args(&args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn();

            let mut child = match child {
                Ok(c) => c,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Failed to spawn '{}': {}",
                        cmd, e
                    )));
                }
            };

            let timeout_duration = Duration::from_millis(timeout_ms);

            // Try to wait for the child with a timeout.
            // We first attempt to wait (non-consuming), then collect output.
            match tokio::time::timeout(timeout_duration, child.wait()).await {
                Ok(Ok(status)) => {
                    // Process finished within timeout — collect output
                    let mut stdout_buf = String::new();
                    let mut stderr_buf = String::new();

                    if let Some(mut out) = child.stdout.take() {
                        use tokio::io::AsyncReadExt;
                        let mut buf = Vec::new();
                        let _ = out.read_to_end(&mut buf).await;
                        stdout_buf = String::from_utf8_lossy(&buf).to_string();
                    }
                    if let Some(mut err) = child.stderr.take() {
                        use tokio::io::AsyncReadExt;
                        let mut buf = Vec::new();
                        let _ = err.read_to_end(&mut buf).await;
                        stderr_buf = String::from_utf8_lossy(&buf).to_string();
                    }

                    let exit_code = status.code().unwrap_or(-1);

                    let mut result = String::new();

                    if !stdout_buf.is_empty() {
                        result.push_str(&stdout_buf);
                    }

                    if !stderr_buf.is_empty() {
                        if !result.is_empty() {
                            result.push_str("\n");
                        }
                        result.push_str("[stderr]\n");
                        result.push_str(&stderr_buf);
                    }

                    if result.is_empty() {
                        result = format!("Process exited with code {}.", exit_code);
                    } else if exit_code != 0 {
                        result.push_str(&format!("\n[exit code: {}]", exit_code));
                    }

                    if status.success() {
                        Ok(ToolResult::success(result))
                    } else {
                        Ok(ToolResult::error(result))
                    }
                }
                Ok(Err(e)) => Ok(ToolResult::error(format!(
                    "Process execution error: {}",
                    e
                ))),
                Err(_) => {
                    // Timeout — try to kill the process
                    let _ = child.kill().await;
                    Ok(ToolResult::error(format!(
                        "Execution timed out after {}ms.",
                        timeout_ms
                    )))
                }
            }
        }
    }
}
