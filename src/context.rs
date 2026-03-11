use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{broadcast, oneshot, Mutex};

use crate::config::ClawConfig;
use crate::memory::MemoryManager;
use crate::scheduler::SchedulerHandle;
use crate::soul::SoulManager;

/// Notification pushed to the web UI via SSE / WebSocket.
#[derive(Clone, Debug, serde::Serialize)]
pub struct NotificationEvent {
    pub id: String,
    pub title: String,
    pub message: String,
    pub level: String,
}

/// A chat message forwarded to a specific web session.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ChatMessageEvent {
    pub session_id: String,
    pub content: String,
}

/// Shared application context that is passed to every tool invocation.
///
/// `ClawContext` is cheaply cloneable (all inner fields are behind `Arc`
/// or are themselves `Clone`-friendly broadcast handles).
#[derive(Clone)]
pub struct ClawContext {
    pub soul: Arc<SoulManager>,
    pub memory: Arc<MemoryManager>,
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub scheduler: Arc<SchedulerHandle>,
    pub notification_tx: broadcast::Sender<NotificationEvent>,
    pub chat_tx: broadcast::Sender<ChatMessageEvent>,
    /// In-flight `ask_user` questions keyed by a unique question ID.
    /// The web handler sends the user's answer through the `oneshot::Sender`.
    pub pending_questions: Arc<DashMap<String, oneshot::Sender<String>>>,
    /// The web session that initiated the current agent run, if any.
    pub session_id: Option<String>,
    pub config: Arc<ClawConfig>,
}
