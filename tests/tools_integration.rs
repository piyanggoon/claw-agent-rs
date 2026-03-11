//! Integration tests for all 21 custom tools.
//!
//! Tests call each tool's `execute()` directly with a real filesystem-backed
//! SoulManager and in-memory SQLite DB — no LLM needed.

use std::sync::Arc;

use agent_sdk::{Tool, ToolContext, ToolResult};
use dashmap::DashMap;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::{broadcast, oneshot, Mutex};

// We need access to the crate's internals
use claw_agent_rs::config::ClawConfig;
use claw_agent_rs::context::{ChatMessageEvent, ClawContext, NotificationEvent};
use claw_agent_rs::db;
use claw_agent_rs::memory::MemoryManager;
use claw_agent_rs::scheduler::SchedulerHandle;
use claw_agent_rs::soul::SoulManager;
use claw_agent_rs::tools;

// ─── Helper: build a test ClawContext with temp directory ─────────────────

fn build_test_context(tmp: &TempDir) -> (ClawContext, broadcast::Receiver<NotificationEvent>, broadcast::Receiver<ChatMessageEvent>) {
    let soul_dir = tmp.path().join("soul");
    std::fs::create_dir_all(soul_dir.join("memory")).unwrap();

    // Seed required soul files
    std::fs::write(soul_dir.join("SOUL.md"), "# Soul\n\n## Personality\n- friendly\n").unwrap();
    std::fs::write(soul_dir.join("IDENTITY.md"), "# Identity\n\n## Name\nTestBot\n").unwrap();
    std::fs::write(soul_dir.join("USER.md"), "# User Profile\n\n## Basics\n- Name: Tester\n").unwrap();
    std::fs::write(soul_dir.join("MEMORY.md"), "# Long-Term Memory\n\n## Key Facts\n- test fact\n\n## Open Loops\n- pending item\n").unwrap();
    std::fs::write(soul_dir.join("HEARTBEAT.md"), "# Heartbeat\n\n## Tasks\n- [@startup] boot\n").unwrap();
    std::fs::write(soul_dir.join("TOOLS.md"), "# Tools\n\n## System\n- OS: test\n").unwrap();
    std::fs::write(soul_dir.join("BOOTSTRAP.md"), "# Bootstrap\nfirst run\n").unwrap();

    let soul = Arc::new(SoulManager::new(&soul_dir));
    let memory = Arc::new(MemoryManager::new(soul.clone()));

    // In-memory SQLite
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;").unwrap();
    db::schema::initialize_db(&conn).unwrap();
    let db = Arc::new(Mutex::new(conn));

    let (notification_tx, notification_rx) = broadcast::channel::<NotificationEvent>(64);
    let (chat_tx, chat_rx) = broadcast::channel::<ChatMessageEvent>(64);
    let scheduler = Arc::new(SchedulerHandle::new());
    let pending_questions: Arc<DashMap<String, oneshot::Sender<String>>> = Arc::new(DashMap::new());

    let config = Arc::new(ClawConfig::from_env());

    let ctx = ClawContext {
        soul,
        memory,
        db,
        scheduler,
        notification_tx,
        chat_tx,
        pending_questions,
        session_id: Some("test-session-001".to_string()),
        config,
        custom_event_tx: None,
    };

    (ctx, notification_rx, chat_rx)
}

/// Helper: execute a tool and assert success
async fn exec_ok(tool: &impl Tool<ClawContext, Name = agent_sdk::DynamicToolName>, ctx: &ToolContext<ClawContext>, input: serde_json::Value) -> ToolResult {
    let result = tool.execute(ctx, input).await.expect("tool execute failed");
    assert!(result.success, "tool returned error: {}", result.output);
    result
}

/// Helper: execute a tool and assert error
async fn exec_err(tool: &impl Tool<ClawContext, Name = agent_sdk::DynamicToolName>, ctx: &ToolContext<ClawContext>, input: serde_json::Value) -> ToolResult {
    let result = tool.execute(ctx, input).await.expect("tool execute panicked");
    assert!(!result.success, "expected error but got success: {}", result.output);
    result
}

// ==========================================================================
// SOUL TOOLS (4)
// ==========================================================================

#[tokio::test]
async fn test_01_soul_read() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::soul::SoulReadTool;
    let result = exec_ok(&tool, &tool_ctx, json!({"filename": "IDENTITY.md"})).await;
    assert!(result.output.contains("TestBot"), "should contain agent name");
}

#[tokio::test]
async fn test_02_soul_update() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx.clone());

    let tool = tools::soul::SoulUpdateTool;
    let result = exec_ok(&tool, &tool_ctx, json!({
        "filename": "TOOLS.md",
        "content": "# Tools\n\n## System\n- OS: macOS\n- Shell: zsh\n"
    })).await;
    assert!(result.output.contains("updated successfully"));

    // Verify
    let content = ctx.soul.read("TOOLS.md").await.unwrap();
    assert!(content.contains("macOS"));
}

#[tokio::test]
async fn test_03_soul_update_section() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx.clone());

    let tool = tools::soul::SoulUpdateSectionTool;
    let result = exec_ok(&tool, &tool_ctx, json!({
        "filename": "SOUL.md",
        "heading": "Personality",
        "content": "- kind\n- curious\n- proactive"
    })).await;
    assert!(result.output.contains("updated successfully"));

    let content = ctx.soul.read("SOUL.md").await.unwrap();
    assert!(content.contains("curious"));
    assert!(!content.contains("friendly"), "old content should be replaced");
}

#[tokio::test]
async fn test_04_soul_delete() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx.clone());

    let tool = tools::soul::SoulDeleteTool;

    // Should succeed for BOOTSTRAP.md
    let result = exec_ok(&tool, &tool_ctx, json!({"filename": "BOOTSTRAP.md"})).await;
    assert!(result.output.contains("deleted"));
    assert!(!ctx.soul.exists("BOOTSTRAP.md"));

    // Should fail for other files
    let result = exec_err(&tool, &tool_ctx, json!({"filename": "SOUL.md"})).await;
    assert!(result.output.to_lowercase().contains("bootstrap"));
}

// ==========================================================================
// MEMORY TOOLS (4)
// ==========================================================================

#[tokio::test]
async fn test_05_memory_save() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx.clone());

    let tool = tools::memory::MemorySaveTool;

    // Append
    let result = exec_ok(&tool, &tool_ctx, json!({
        "section": "Key Facts",
        "content": "- timezone: Asia/Bangkok"
    })).await;
    assert!(result.output.contains("saved"));

    let content = ctx.soul.read("MEMORY.md").await.unwrap();
    assert!(content.contains("timezone: Asia/Bangkok"), "appended content should exist");
    assert!(content.contains("test fact"), "original content should still exist");

    // Replace
    let result = exec_ok(&tool, &tool_ctx, json!({
        "section": "Key Facts",
        "content": "- only this remains",
        "action": "replace"
    })).await;
    assert!(result.output.contains("saved"));

    let content = ctx.soul.read("MEMORY.md").await.unwrap();
    assert!(content.contains("only this remains"));
}

#[tokio::test]
async fn test_06_memory_daily_log() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx.clone());

    let tool = tools::memory::MemoryDailyLogTool;

    let result = exec_ok(&tool, &tool_ctx, json!({
        "content": "User asked about Rust testing",
        "category": "interaction"
    })).await;
    assert!(result.output.contains("added"));

    // Verify daily log file exists
    let logs = ctx.memory.get_recent_daily_logs(1).await.unwrap();
    assert!(!logs.is_empty(), "should have at least one daily log");
    assert!(logs[0].1.contains("Rust testing"));
}

#[tokio::test]
async fn test_07_memory_recall() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx.clone());

    // First write some memories
    let save_tool = tools::memory::MemorySaveTool;
    exec_ok(&save_tool, &tool_ctx, json!({
        "section": "Key Facts",
        "content": "- user timezone is UTC+7"
    })).await;

    let tool = tools::memory::MemoryRecallTool;

    // Search with query
    let result = exec_ok(&tool, &tool_ctx, json!({"query": "timezone"})).await;
    assert!(result.output.contains("UTC+7") || result.output.contains("timezone"));

    // Search without query (return all)
    let result = exec_ok(&tool, &tool_ctx, json!({})).await;
    assert!(!result.output.is_empty());
}

#[tokio::test]
async fn test_08_memory_forget() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx.clone());

    let tool = tools::memory::MemoryForgetTool;

    let result = exec_ok(&tool, &tool_ctx, json!({
        "section": "Key Facts",
        "entry": "test fact"
    })).await;
    assert!(result.output.to_lowercase().contains("removed") || result.output.to_lowercase().contains("forgot"));

    let content = ctx.soul.read("MEMORY.md").await.unwrap();
    assert!(!content.contains("test fact"), "forgotten entry should be removed");
}

// ==========================================================================
// HEARTBEAT TOOLS (2)
// ==========================================================================

#[tokio::test]
async fn test_09_heartbeat_read() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::heartbeat::HeartbeatReadTool;
    let result = exec_ok(&tool, &tool_ctx, json!({})).await;
    assert!(result.output.contains("@startup"), "should contain heartbeat tasks");
}

#[tokio::test]
async fn test_10_heartbeat_update() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx.clone());

    let tool = tools::heartbeat::HeartbeatUpdateTool;
    let result = exec_ok(&tool, &tool_ctx, json!({
        "content": "# Heartbeat\n\n## Tasks\n- [@daily 09:00] Good morning check\n"
    })).await;
    assert!(result.output.contains("updated"));

    let content = ctx.soul.read("HEARTBEAT.md").await.unwrap();
    assert!(content.contains("Good morning check"));
}

// ==========================================================================
// TASK TOOLS (5)
// ==========================================================================

#[tokio::test]
async fn test_11_schedule_and_list_tasks() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    // Schedule a delay task
    let schedule_tool = tools::tasks::ScheduleTaskTool;
    let result = exec_ok(&schedule_tool, &tool_ctx, json!({
        "prompt": "Say hello",
        "schedule_type": "delay",
        "schedule_value": "60000"
    })).await;
    assert!(result.output.contains("task_id"));
    let task_id: String = serde_json::from_str::<serde_json::Value>(&result.output)
        .unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_string();

    // List tasks
    let list_tool = tools::tasks::ListTasksTool;
    let result = exec_ok(&list_tool, &tool_ctx, json!({})).await;
    assert!(result.output.contains(&task_id), "listed tasks should contain our task");

    // Return task_id for reuse
    assert!(!task_id.is_empty());
}

#[tokio::test]
async fn test_12_pause_resume_cancel_task() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    // Create task first
    let schedule_tool = tools::tasks::ScheduleTaskTool;
    let result = exec_ok(&schedule_tool, &tool_ctx, json!({
        "prompt": "Test task",
        "schedule_type": "delay",
        "schedule_value": "300000"
    })).await;
    let task_id: String = serde_json::from_str::<serde_json::Value>(&result.output)
        .unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Pause
    let pause_tool = tools::tasks::PauseTaskTool;
    let result = exec_ok(&pause_tool, &tool_ctx, json!({"task_id": task_id})).await;
    assert!(result.output.to_lowercase().contains("paused"));

    // Resume
    let resume_tool = tools::tasks::ResumeTaskTool;
    let result = exec_ok(&resume_tool, &tool_ctx, json!({"task_id": task_id})).await;
    assert!(result.output.to_lowercase().contains("resumed"));

    // Cancel
    let cancel_tool = tools::tasks::CancelTaskTool;
    let result = exec_ok(&cancel_tool, &tool_ctx, json!({"task_id": task_id})).await;
    assert!(result.output.to_lowercase().contains("cancel"));

    // Verify deleted
    let list_tool = tools::tasks::ListTasksTool;
    let result = exec_ok(&list_tool, &tool_ctx, json!({})).await;
    assert!(!result.output.contains(&task_id), "cancelled task should be gone");
}

// ==========================================================================
// UTILITY TOOLS (6)
// ==========================================================================

#[tokio::test]
async fn test_13_send_notification() {
    let tmp = TempDir::new().unwrap();
    let (ctx, mut rx, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::utility::SendNotificationTool;
    let result = exec_ok(&tool, &tool_ctx, json!({
        "title": "Test Alert",
        "message": "Something happened",
        "level": "warning"
    })).await;
    assert!(result.output.to_lowercase().contains("sent") || result.output.to_lowercase().contains("notification"));

    // Check broadcast received
    let notif = rx.try_recv();
    assert!(notif.is_ok(), "should have received notification via broadcast");
    let notif = notif.unwrap();
    assert_eq!(notif.title, "Test Alert");
    assert_eq!(notif.level, "warning");
}

#[tokio::test]
async fn test_14_send_chat_message() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, mut chat_rx) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::utility::SendChatMessageTool;
    let result = exec_ok(&tool, &tool_ctx, json!({
        "content": "Hello from background task!"
    })).await;
    assert!(result.output.to_lowercase().contains("sent") || result.output.to_lowercase().contains("message"));

    let msg = chat_rx.try_recv();
    assert!(msg.is_ok(), "should have received chat message via broadcast");
    assert!(msg.unwrap().content.contains("Hello from background"));
}

#[tokio::test]
async fn test_15_ask_user() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);

    let pending = ctx.pending_questions.clone();
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::utility::AskUserTool;

    // Spawn a task that polls pending_questions until a question appears, then answers it
    let pending_clone = pending.clone();
    tokio::spawn(async move {
        // Poll until a question appears (up to 5 seconds)
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if !pending_clone.is_empty() {
                let key = pending_clone.iter().next().unwrap().key().clone();
                if let Some((_, sender)) = pending_clone.remove(&key) {
                    let _ = sender.send("Yes, proceed!".to_string());
                }
                return;
            }
        }
    });

    let result = exec_ok(&tool, &tool_ctx, json!({
        "question": "Should I continue?",
        "options": ["Yes", "No"]
    })).await;
    assert!(result.output.contains("Yes, proceed!"));
}

#[tokio::test]
async fn test_16_web_fetch() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::utility::WebFetchTool;
    let result = exec_ok(&tool, &tool_ctx, json!({
        "url": "https://httpbin.org/get",
        "max_length": 1000
    })).await;
    assert!(result.output.contains("httpbin") || result.output.contains("origin") || result.output.contains("headers"));
}

#[tokio::test]
async fn test_17_code_execute_bash() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::utility::CodeExecuteTool;
    let result = exec_ok(&tool, &tool_ctx, json!({
        "language": "bash",
        "code": "echo 'hello from bash' && echo $((2+3))"
    })).await;
    assert!(result.output.contains("hello from bash"));
    assert!(result.output.contains("5"));
}

#[tokio::test]
async fn test_18_code_execute_python() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::utility::CodeExecuteTool;
    let result = exec_ok(&tool, &tool_ctx, json!({
        "language": "python",
        "code": "print('hello from python')\nprint(2**10)"
    })).await;
    assert!(result.output.contains("hello from python"));
    assert!(result.output.contains("1024"));
}

#[tokio::test]
async fn test_19_code_execute_javascript() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::utility::CodeExecuteTool;
    let result = exec_ok(&tool, &tool_ctx, json!({
        "language": "javascript",
        "code": "console.log('hello from js'); console.log(JSON.stringify({a:1}))"
    })).await;
    assert!(result.output.contains("hello from js"));
}

#[tokio::test]
async fn test_20_run_background() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::utility::RunBackgroundTool;
    let result = exec_ok(&tool, &tool_ctx, json!({
        "prompt": "Say hello in background"
    })).await;
    // run_background creates a task in DB and returns task_id
    assert!(result.output.contains("task_id") || result.output.to_lowercase().contains("background"));
}

// ==========================================================================
// ERROR CASES
// ==========================================================================

#[tokio::test]
async fn test_21_soul_read_missing_file() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::soul::SoulReadTool;
    let result = exec_err(&tool, &tool_ctx, json!({"filename": "NONEXISTENT.md"})).await;
    assert!(!result.output.is_empty());
}

#[tokio::test]
async fn test_22_schedule_task_invalid_type() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _, _) = build_test_context(&tmp);
    let tool_ctx = ToolContext::new(ctx);

    let tool = tools::tasks::ScheduleTaskTool;
    let result = exec_err(&tool, &tool_ctx, json!({
        "prompt": "fail",
        "schedule_type": "invalid_type",
        "schedule_value": "123"
    })).await;
    assert!(result.output.to_lowercase().contains("unknown") || result.output.to_lowercase().contains("error"));
}
