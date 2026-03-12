use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

use claw_agent_rs::config::ClawConfig;
use claw_agent_rs::context::{ChatMessageEvent, NotificationEvent};
use claw_agent_rs::db;
use claw_agent_rs::memory::MemoryManager;
use claw_agent_rs::scheduler::{SchedulerHandle, TaskScheduler};
use claw_agent_rs::soul::SoulManager;
use claw_agent_rs::web;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "claw_agent_rs=info,tower_http=info".into()),
        )
        .init();

    // Load environment variables from .env
    let _ = dotenvy::dotenv();

    // Load configuration
    let config = Arc::new(ClawConfig::from_env());
    tracing::info!(
        port = config.web_port,
        model = %config.default_model,
        "Starting Claw Agent RS"
    );

    // Ensure data directory exists
    std::fs::create_dir_all(&config.data_dir)?;

    // Ensure uploads directory exists
    let uploads_dir = config.data_dir.join("uploads");
    std::fs::create_dir_all(&uploads_dir)?;

    // Initialize SQLite database
    let db_path = config.data_dir.join("claw.db");
    let conn = rusqlite::Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    db::schema::initialize_db(&conn)?;
    // Migrate old RFC 3339 timestamps (+00:00) to JS-compatible format (Z)
    db::messages::migrate_timestamps(&conn)?;
    let db = Arc::new(Mutex::new(conn));
    tracing::info!(path = %db_path.display(), "Database initialized");

    // Initialize group directory — copy from default template if group doesn't exist yet
    let group_dir = config.groups_dir.join(&config.main_group);
    let soul_dir = config.soul_dir();
    if !soul_dir.exists() {
        let default_dir = config.groups_dir.join("default");
        let default_soul = default_dir.join("soul");
        anyhow::ensure!(
            default_soul.exists(),
            "Default soul template not found at {}",
            default_soul.display()
        );
        // Copy soul/*.md files
        std::fs::create_dir_all(&soul_dir)?;
        for entry in std::fs::read_dir(&default_soul)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                std::fs::copy(&path, soul_dir.join(entry.file_name()))?;
            }
        }
        // Copy group-level files (AGENTS.md, etc.)
        for entry in std::fs::read_dir(&default_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                std::fs::copy(&path, group_dir.join(entry.file_name()))?;
            }
        }
        tracing::info!(
            group = %config.main_group,
            "Created new group from default template"
        );
    }
    std::fs::create_dir_all(soul_dir.join("memory"))?;
    let soul = Arc::new(SoulManager::new(soul_dir));

    // Initialize memory manager
    let memory = Arc::new(MemoryManager::new(soul.clone()));

    // Create broadcast channels
    let (notification_tx, _) = broadcast::channel::<NotificationEvent>(256);
    let (chat_tx, _) = broadcast::channel::<ChatMessageEvent>(256);
    let (task_events_tx, _) = broadcast::channel::<serde_json::Value>(256);

    // Create scheduler handle
    let scheduler_handle = Arc::new(SchedulerHandle::new());

    // Build AppState for web server
    let app_state = web::state::AppState {
        db: db.clone(),
        soul: soul.clone(),
        memory: memory.clone(),
        config: config.clone(),
        scheduler: scheduler_handle.clone(),
        active_runs: Arc::new(dashmap::DashMap::new()),
        abort_handles: Arc::new(dashmap::DashMap::new()),
        notification_tx: notification_tx.clone(),
        chat_tx: chat_tx.clone(),
        pending_questions: Arc::new(dashmap::DashMap::new()),
        run_sessions: Arc::new(dashmap::DashMap::new()),
        custom_events: Arc::new(dashmap::DashMap::new()),
        task_events_tx: task_events_tx.clone(),
        run_accumulators: Arc::new(dashmap::DashMap::new()),
    };

    // Start task scheduler
    let task_scheduler = Arc::new(TaskScheduler::new(
        db.clone(),
        config.clone(),
        soul.clone(),
        memory.clone(),
        scheduler_handle.clone(),
        notification_tx.clone(),
        chat_tx.clone(),
    ));
    let _scheduler_handle = task_scheduler.start();
    tracing::info!(
        poll_interval_secs = config.scheduler_poll_interval.as_secs(),
        "Task scheduler started"
    );

    // Start web server (this blocks until shutdown)
    let port = config.web_port;
    web::server::start_server(app_state, port).await?;

    Ok(())
}
