use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, Notify};
use tokio::task::JoinHandle;

use crate::config::ClawConfig;
use crate::context::{ClawContext, NotificationEvent, ChatMessageEvent};
use crate::db;
use crate::soul::SoulManager;
use crate::memory::MemoryManager;
use crate::agent::runner::run_agent;
use agent_sdk::ThreadId;

/// Handle for communicating with the scheduler from tools
pub struct SchedulerHandle {
    notify: Notify,
}

impl SchedulerHandle {
    pub fn new() -> Self {
        Self { notify: Notify::new() }
    }

    /// Notify scheduler that a new task was created (wake up immediately)
    pub fn notify_new_task(&self) {
        self.notify.notify_one();
    }
}

pub struct TaskScheduler {
    db: Arc<Mutex<rusqlite::Connection>>,
    config: Arc<ClawConfig>,
    soul: Arc<SoulManager>,
    memory: Arc<MemoryManager>,
    scheduler_handle: Arc<SchedulerHandle>,
    notification_tx: broadcast::Sender<NotificationEvent>,
    chat_tx: broadcast::Sender<ChatMessageEvent>,
}

impl TaskScheduler {
    pub fn new(
        db: Arc<Mutex<rusqlite::Connection>>,
        config: Arc<ClawConfig>,
        soul: Arc<SoulManager>,
        memory: Arc<MemoryManager>,
        scheduler_handle: Arc<SchedulerHandle>,
        notification_tx: broadcast::Sender<NotificationEvent>,
        chat_tx: broadcast::Sender<ChatMessageEvent>,
    ) -> Self {
        Self { db, config, soul, memory, scheduler_handle, notification_tx, chat_tx }
    }

    /// Start the scheduler loop. Returns join handle.
    pub fn start(self: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                self.poll_and_execute().await;
                // Wait for poll interval OR notification of new task
                tokio::select! {
                    _ = tokio::time::sleep(self.config.scheduler_poll_interval) => {},
                    _ = self.scheduler_handle.notify.notified() => {
                        tracing::debug!("Scheduler woken up by new task notification");
                    },
                }
            }
        })
    }

    async fn poll_and_execute(&self) {
        let due_tasks = {
            let db = self.db.lock().await;
            match db::tasks::get_due_tasks(&db) {
                Ok(tasks) => tasks,
                Err(e) => {
                    tracing::error!(err = %e, "Failed to get due tasks");
                    return;
                }
            }
        };

        if due_tasks.is_empty() {
            return;
        }

        tracing::info!(count = due_tasks.len(), "Found due tasks");

        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.max_concurrent_tasks));
        let mut handles = Vec::new();

        for task in due_tasks {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let this = self.clone_inner();

            handles.push(tokio::spawn(async move {
                this.run_task(&task).await;
                drop(permit);
            }));
        }

        futures::future::join_all(handles).await;
    }

    async fn run_task(&self, task: &db::tasks::ScheduledTask) {
        let start = std::time::Instant::now();

        // Mark task as running
        {
            let db = self.db.lock().await;
            let _ = db::tasks::set_task_running(&db, &task.id);
        }

        // Build context for this task
        let pending_questions = Arc::new(dashmap::DashMap::new());
        let ctx = ClawContext {
            soul: self.soul.clone(),
            memory: self.memory.clone(),
            db: self.db.clone(),
            scheduler: self.scheduler_handle.clone(),
            notification_tx: self.notification_tx.clone(),
            chat_tx: self.chat_tx.clone(),
            pending_questions,
            session_id: None,
            config: self.config.clone(),
            custom_event_tx: None,
        };

        // Determine thread_id based on context mode
        let thread_id = if task.context_mode == "group" {
            task.context_session.as_ref()
                .map(|s| ThreadId::from_string(s.clone()))
                .unwrap_or_else(ThreadId::new)
        } else {
            ThreadId::new()
        };

        let prompt = format!("[SCHEDULED TASK]\n\n{}", task.prompt);
        let (event_tx, _) = broadcast::channel(256);

        // Run agent
        let result = run_agent(ctx, thread_id, prompt, None, event_tx).await;
        let duration = start.elapsed();

        // Update task after run
        let (status, result_text, error_text) = match &result {
            Ok(run_result) => {
                let summary = if run_result.accumulated_text.is_empty() {
                    "Task completed successfully".to_string()
                } else {
                    let text = &run_result.accumulated_text;
                    if text.len() > 500 {
                        format!("{}...", &text[..500])
                    } else {
                        text.clone()
                    }
                };
                ("success", Some(summary), None)
            }
            Err(e) => ("error", None, Some(e.to_string())),
        };

        // Calculate next_run
        let next_run = calculate_next_run(&task.schedule_type, &task.schedule_value);

        {
            let db = self.db.lock().await;
            let _ = db::tasks::update_task_after_run(
                &db,
                &task.id,
                next_run.as_deref(),
                result_text.as_deref(),
            );
            let _ = db::tasks::log_task_run(
                &db,
                &task.id,
                duration.as_millis() as u64,
                status,
                result_text.as_deref(),
                error_text.as_deref(),
            );
        }

        tracing::info!(
            task_id = task.id,
            status,
            duration_ms = duration.as_millis() as u64,
            "Task execution completed"
        );
    }

    fn clone_inner(&self) -> Self {
        Self {
            db: self.db.clone(),
            config: self.config.clone(),
            soul: self.soul.clone(),
            memory: self.memory.clone(),
            scheduler_handle: self.scheduler_handle.clone(),
            notification_tx: self.notification_tx.clone(),
            chat_tx: self.chat_tx.clone(),
        }
    }
}

/// Calculate next_run based on schedule type
pub fn calculate_next_run(schedule_type: &str, schedule_value: &str) -> Option<String> {
    match schedule_type {
        "cron" => {
            use std::str::FromStr;
            let schedule = cron::Schedule::from_str(schedule_value).ok()?;
            let next = schedule.upcoming(chrono::Utc).next()?;
            Some(next.to_rfc3339())
        }
        "interval" => {
            let ms: u64 = schedule_value.parse().ok()?;
            let next = chrono::Utc::now() + chrono::Duration::milliseconds(ms as i64);
            Some(next.to_rfc3339())
        }
        "once" | "delay" => None, // one-shot, no next run
        _ => None,
    }
}

/// Calculate initial next_run when creating a task
pub fn calculate_initial_next_run(schedule_type: &str, schedule_value: &str) -> Option<String> {
    match schedule_type {
        "delay" => {
            let ms: u64 = schedule_value.parse().ok()?;
            let next = chrono::Utc::now() + chrono::Duration::milliseconds(ms as i64);
            Some(next.to_rfc3339())
        }
        "once" => Some(schedule_value.to_string()),
        "interval" => {
            let ms: u64 = schedule_value.parse().ok()?;
            let next = chrono::Utc::now() + chrono::Duration::milliseconds(ms as i64);
            Some(next.to_rfc3339())
        }
        "cron" => {
            use std::str::FromStr;
            let schedule = cron::Schedule::from_str(schedule_value).ok()?;
            let next = schedule.upcoming(chrono::Utc).next()?;
            Some(next.to_rfc3339())
        }
        _ => None,
    }
}
