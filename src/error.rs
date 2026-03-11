use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClawError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Soul file not found: {0}")]
    SoulFileNotFound(String),

    #[error("Operation not allowed: {0}")]
    NotAllowed(String),

    #[error("Task not found: {0}")]
    TaskNotFound(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Agent error: {0}")]
    Agent(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ClawError>;
