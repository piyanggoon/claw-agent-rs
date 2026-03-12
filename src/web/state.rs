use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{broadcast, oneshot, Mutex, RwLock};
use tokio::task::AbortHandle;

use agent_sdk::AgentEventEnvelope;
use crate::config::ClawConfig;
use crate::context::{ChatMessageEvent, NotificationEvent};
use crate::memory::MemoryManager;
use crate::scheduler::SchedulerHandle;
use crate::soul::SoulManager;

/// Accumulated state for a running agent — used to replay events on SSE reconnection.
pub struct RunAccumulator {
    /// Text accumulated from TextDelta events.
    pub text: String,
    /// Tool calls accumulated during the run (with contentSplitIndex).
    pub tool_calls: Vec<serde_json::Value>,
}

impl Default for RunAccumulator {
    fn default() -> Self {
        Self {
            text: String::new(),
            tool_calls: Vec::new(),
        }
    }
}

/// Shared application state passed to all Axum route handlers.
///
/// Every field is cheaply cloneable (`Arc` or broadcast handles), so cloning
/// `AppState` for each request is essentially free.
#[derive(Clone)]
pub struct AppState {
    /// SQLite database connection, protected by an async mutex.
    pub db: Arc<Mutex<rusqlite::Connection>>,

    /// Manages reading and writing soul files on disk.
    pub soul: Arc<SoulManager>,

    /// Manages structured memory (MEMORY.md) and daily logs.
    pub memory: Arc<MemoryManager>,

    /// Application configuration loaded from the environment.
    pub config: Arc<ClawConfig>,

    /// Handle to the background task scheduler.
    pub scheduler: Arc<SchedulerHandle>,

    /// Active agent runs: `run_id` -> broadcast sender for streaming events.
    ///
    /// When a chat request spawns an agent, the agent's event stream is
    /// published through the broadcast channel stored here. SSE clients
    /// subscribe to this channel to receive real-time events.
    pub active_runs: Arc<DashMap<String, broadcast::Sender<AgentEventEnvelope>>>,

    /// Abort handles for stopping in-flight agent runs.
    pub abort_handles: Arc<DashMap<String, AbortHandle>>,

    /// Broadcast channel for push notifications to the web UI.
    pub notification_tx: broadcast::Sender<NotificationEvent>,

    /// Broadcast channel for chat messages pushed from background tasks.
    pub chat_tx: broadcast::Sender<ChatMessageEvent>,

    /// Pending `ask_user` questions keyed by a unique question ID.
    ///
    /// The web handler resolves the question by sending the user's answer
    /// through the contained `oneshot::Sender`.
    pub pending_questions: Arc<DashMap<String, oneshot::Sender<String>>>,

    /// Maps `run_id` -> `session_id` for chat status reporting.
    pub run_sessions: Arc<DashMap<String, String>>,

    /// Custom SSE event channels per run (for ask_user, plan_update, etc.).
    pub custom_events: Arc<DashMap<String, broadcast::Sender<serde_json::Value>>>,

    /// Broadcast channel for real-time task lifecycle events (SSE to frontend).
    pub task_events_tx: broadcast::Sender<serde_json::Value>,

    /// Per-run accumulated text + tool calls for SSE reconnection replay.
    pub run_accumulators: Arc<DashMap<String, Arc<RwLock<RunAccumulator>>>>,
}
