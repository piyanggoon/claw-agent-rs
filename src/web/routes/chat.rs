use std::convert::Infallible;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::web::state::AppState;

// ── Request / Response types ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub session_id: Option<String>,
    pub model: Option<String>,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub run_id: String,
    pub session_id: String,
}

#[derive(Deserialize)]
pub struct RespondRequest {
    pub question_id: String,
    pub answer: String,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// POST /api/chat
///
/// Accepts a user message, creates (or reuses) a web session, stores the user
/// message in the database, and spawns a background tokio task that runs the
/// agent loop. Returns `{ run_id, session_id }` immediately so the client can
/// connect to the SSE stream.
pub async fn create_chat(
    State(state): State<AppState>,
    Json(payload): Json<ChatRequest>,
) -> Result<(StatusCode, Json<ChatResponse>), (StatusCode, String)> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let session_id = payload
        .session_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Ensure the web session exists in the database.
    {
        let db = state.db.lock().await;
        let existing = crate::db::sessions::get_session(&db, &session_id)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if existing.is_none() {
            crate::db::sessions::create_session(&db, &session_id, None)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
    }

    // Store the user message.
    {
        let msg_id = uuid::Uuid::new_v4().to_string();
        let db = state.db.lock().await;
        crate::db::messages::store_message(
            &db,
            &msg_id,
            &run_id,      // thread_id
            "user",
            &payload.message,
            Some(&session_id),
            None,
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        // Touch session so it floats to the top of the list.
        let _ = crate::db::sessions::touch_session(&db, &session_id);
    }

    // Create a broadcast channel for this run's events.
    let (tx, _rx) = tokio::sync::broadcast::channel(256);
    state.active_runs.insert(run_id.clone(), tx.clone());

    // Spawn the agent loop in a background task.
    let run_id_clone = run_id.clone();
    let session_id_clone = session_id.clone();
    let state_clone = state.clone();
    let message = payload.message.clone();
    let model = payload.model.clone();

    let handle = tokio::spawn(async move {
        // Build a ClawContext from the AppState for the agent run.
        let ctx = crate::context::ClawContext {
            soul: state_clone.soul.clone(),
            memory: state_clone.memory.clone(),
            db: state_clone.db.clone(),
            scheduler: state_clone.scheduler.clone(),
            notification_tx: state_clone.notification_tx.clone(),
            chat_tx: state_clone.chat_tx.clone(),
            pending_questions: state_clone.pending_questions.clone(),
            session_id: Some(session_id_clone.clone()),
            config: state_clone.config.clone(),
        };
        let thread_id = agent_sdk::ThreadId::from_string(run_id_clone.clone());
        let result = crate::agent::runner::run_agent(
            ctx,
            thread_id,
            message,
            model,
            tx,
        )
        .await;

        if let Err(e) = &result {
            tracing::error!(run_id = %run_id_clone, "agent run failed: {e}");
        }

        // Clean up the active run entry when the agent finishes.
        state_clone.active_runs.remove(&run_id_clone);
        state_clone.abort_handles.remove(&run_id_clone);
    });

    // Store the abort handle so we can cancel the run if requested.
    state.abort_handles.insert(run_id.clone(), handle.abort_handle());

    Ok((
        StatusCode::CREATED,
        Json(ChatResponse {
            run_id,
            session_id,
        }),
    ))
}

/// GET /api/chat/stream/:run_id
///
/// Subscribes to the broadcast channel for the given `run_id` and returns an
/// SSE stream of `AgentEventEnvelope` events serialized as JSON. The stream
/// ends when the agent finishes (the broadcast sender is dropped).
pub async fn stream_events(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let tx = state
        .active_runs
        .get(&run_id)
        .ok_or((StatusCode::NOT_FOUND, format!("run {run_id} not found")))?
        .value()
        .clone();

    let rx = tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(envelope) => {
            let data = serde_json::to_string(&envelope).unwrap_or_default();
            Some(Ok(Event::default().data(data)))
        }
        // Lagged means the client fell behind; skip and continue.
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => None,
    });

    Ok(Sse::new(stream))
}

/// POST /api/chat/respond
///
/// Resolves a pending `ask_user` question. The agent is blocked on a
/// `oneshot::Receiver`; sending the answer through the corresponding
/// `oneshot::Sender` unblocks it.
pub async fn respond_to_question(
    State(state): State<AppState>,
    Json(payload): Json<RespondRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let sender = state
        .pending_questions
        .remove(&payload.question_id)
        .map(|(_, sender)| sender)
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("question {} not found", payload.question_id),
        ))?;

    sender.send(payload.answer).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to send answer (receiver dropped)".to_string(),
        )
    })?;

    Ok(StatusCode::OK)
}

/// POST /api/chat/stop/:run_id
///
/// Aborts a running agent by cancelling its tokio task.
pub async fn stop_agent(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let handle = state
        .abort_handles
        .get(&run_id)
        .ok_or((StatusCode::NOT_FOUND, format!("run {run_id} not found")))?
        .value()
        .clone();

    handle.abort();

    // Clean up maps immediately.
    state.abort_handles.remove(&run_id);
    state.active_runs.remove(&run_id);

    Ok(StatusCode::OK)
}
