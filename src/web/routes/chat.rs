use std::convert::Infallible;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::json;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::db::{messages, sessions};
use crate::web::sse;
use crate::web::state::AppState;

// ── Request types ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(rename = "newSession")]
    pub new_session: Option<bool>,
    #[serde(rename = "idempotencyKey")]
    pub idempotency_key: Option<String>,
    #[serde(rename = "webSessionId")]
    pub web_session_id: Option<String>,
    pub images: Option<Vec<String>>,
    #[serde(rename = "planMode")]
    pub plan_mode: Option<bool>,
    pub model: Option<String>,
    pub group: Option<String>,
    pub mode: Option<String>,
}

#[derive(Deserialize)]
pub struct RespondRequest {
    pub question_id: String,
    pub response: String,
    pub run_id: Option<String>,
}

#[derive(Deserialize)]
pub struct StopRequest {
    #[serde(rename = "runId")]
    pub run_id: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// POST /api/chat
///
/// Accepts a user message, creates or reuses a session, stores the message,
/// spawns the agent, and returns an SSE stream directly as the response body.
/// The first event is always `web_session_id`.
pub async fn create_chat(
    State(state): State<AppState>,
    Json(payload): Json<ChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let run_id = uuid::Uuid::new_v4().to_string();

    // Determine session_id
    let session_id = if payload.new_session.unwrap_or(false) || payload.web_session_id.is_none() {
        let id = uuid::Uuid::new_v4().to_string();
        let db = state.db.lock().await;
        sessions::create_session(&db, &id, None)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        id
    } else {
        let id = payload.web_session_id.clone().unwrap();
        let db = state.db.lock().await;
        let existing = sessions::get_session(&db, &id)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if existing.is_none() {
            sessions::create_session(&db, &id, None)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        id
    };

    // Build message text with image references
    let mut message_text = payload.message.clone();
    if let Some(images) = &payload.images {
        for img in images {
            message_text.push_str(&format!("\n[User attached image: {img}]"));
        }
    }

    // Prepend plan mode instruction
    if payload.plan_mode.unwrap_or(false) {
        message_text = format!(
            "[SYSTEM INSTRUCTION - PLAN MODE: Present your plan first, then wait for approval before implementing.]\n\n{message_text}"
        );
    }

    // Store the user message
    {
        let msg_id = uuid::Uuid::new_v4().to_string();
        let db = state.db.lock().await;
        messages::store_message(&db, &msg_id, &run_id, "user", &payload.message, Some(&session_id), None)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let _ = sessions::touch_session(&db, &session_id);
    }

    // Create broadcast channels
    let (event_tx, _) = tokio::sync::broadcast::channel(256);
    let (custom_tx, _) = tokio::sync::broadcast::channel::<serde_json::Value>(64);

    // Subscribe BEFORE spawning so we don't miss events
    let agent_rx = event_tx.subscribe();
    let custom_rx = custom_tx.subscribe();

    // Store in maps for status/reconnection
    state.active_runs.insert(run_id.clone(), event_tx.clone());
    state.custom_events.insert(run_id.clone(), custom_tx.clone());
    state.run_sessions.insert(run_id.clone(), session_id.clone());

    // Spawn the agent loop
    let run_id_clone = run_id.clone();
    let session_id_clone = session_id.clone();
    let state_clone = state.clone();
    let model = payload.model.clone();

    let handle = tokio::spawn(async move {
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
            custom_event_tx: Some(custom_tx),
        };
        let thread_id = agent_sdk::ThreadId::from_string(run_id_clone.clone());
        let result = crate::agent::runner::run_agent(ctx, thread_id, message_text, model, event_tx).await;

        // Store assistant message with metadata
        if let Ok(run_result) = &result {
            let cost = sse::estimate_cost(run_result.input_tokens, run_result.output_tokens);
            let metadata = json!({
                "toolCalls": run_result.tool_calls,
                "resultMeta": {
                    "costUsd": cost,
                    "durationMs": run_result.duration_ms,
                    "numTurns": run_result.total_turns,
                    "inputTokens": run_result.input_tokens,
                    "outputTokens": run_result.output_tokens,
                    "cacheReadTokens": 0,
                    "cacheCreationTokens": 0,
                }
            });
            let msg_id = uuid::Uuid::new_v4().to_string();
            let db = state_clone.db.lock().await;
            let _ = messages::store_message(
                &db, &msg_id, &run_id_clone, "assistant",
                &run_result.accumulated_text, Some(&session_id_clone),
                Some(&metadata.to_string()),
            );
        } else if let Err(e) = &result {
            tracing::error!(run_id = %run_id_clone, "agent run failed: {e}");
        }

        // Clean up
        state_clone.active_runs.remove(&run_id_clone);
        state_clone.custom_events.remove(&run_id_clone);
        state_clone.abort_handles.remove(&run_id_clone);
        state_clone.run_sessions.remove(&run_id_clone);
    });

    state.abort_handles.insert(run_id.clone(), handle.abort_handle());

    // Build SSE stream: transform agent events + pass through custom events
    let agent_stream = BroadcastStream::new(agent_rx).filter_map(|result| match result {
        Ok(envelope) => sse::transform_event(&envelope).map(|json| {
            let data = serde_json::to_string(&json).unwrap_or_default();
            Ok(Event::default().data(data))
        }),
        Err(_) => None,
    });

    let custom_stream = BroadcastStream::new(custom_rx).filter_map(|result| match result {
        Ok(json) => {
            let data = serde_json::to_string(&json).unwrap_or_default();
            Some(Ok(Event::default().data(data)))
        }
        Err(_) => None,
    });

    let merged = agent_stream.merge(custom_stream);

    // Prepend web_session_id as first event
    let initial_data = json!({"type": "web_session_id", "web_session_id": session_id});
    let initial = futures::stream::once(async move {
        Ok(Event::default().data(serde_json::to_string(&initial_data).unwrap()))
    });

    Ok(Sse::new(initial.chain(merged)).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

/// GET /api/chat/status
pub async fn chat_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let runs: Vec<serde_json::Value> = state.run_sessions.iter()
        .map(|e| json!({"runId": e.key(), "sessionId": e.value()}))
        .collect();
    Json(json!({"running": !runs.is_empty(), "runs": runs}))
}

/// GET /api/chat/stream/:run_id — SSE reconnection
pub async fn stream_events(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let event_tx = state.active_runs.get(&run_id)
        .ok_or((StatusCode::NOT_FOUND, format!("run {run_id} not found")))?
        .value().clone();

    let agent_rx = event_tx.subscribe();
    let agent_stream = BroadcastStream::new(agent_rx).filter_map(|result| match result {
        Ok(envelope) => sse::transform_event(&envelope).map(|json| {
            let data = serde_json::to_string(&json).unwrap_or_default();
            Ok(Event::default().data(data))
        }),
        Err(_) => None,
    });

    Ok(Sse::new(agent_stream))
}

/// POST /api/chat/respond
pub async fn respond_to_question(
    State(state): State<AppState>,
    Json(payload): Json<RespondRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let sender = state.pending_questions
        .remove(&payload.question_id)
        .map(|(_, s)| s)
        .ok_or((StatusCode::NOT_FOUND, format!("question {} not found", payload.question_id)))?;

    sender.send(payload.response).map_err(|_| {
        (StatusCode::INTERNAL_SERVER_ERROR, "failed to send answer (receiver dropped)".to_string())
    })?;

    Ok(Json(json!({"ok": true})))
}

/// POST /api/chat/stop
pub async fn stop_agent(
    State(state): State<AppState>,
    Json(payload): Json<StopRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let run_id = if let Some(rid) = payload.run_id {
        rid
    } else if let Some(sid) = &payload.session_id {
        state.run_sessions.iter()
            .find(|e| e.value() == sid)
            .map(|e| e.key().clone())
            .ok_or((StatusCode::NOT_FOUND, format!("no active run for session {sid}")))?
    } else {
        state.abort_handles.iter().next()
            .map(|e| e.key().clone())
            .ok_or((StatusCode::NOT_FOUND, "no active runs".to_string()))?
    };

    let handle = state.abort_handles.get(&run_id)
        .ok_or((StatusCode::NOT_FOUND, format!("run {run_id} not found")))?
        .value().clone();

    handle.abort();
    state.abort_handles.remove(&run_id);
    state.active_runs.remove(&run_id);
    state.custom_events.remove(&run_id);
    state.run_sessions.remove(&run_id);

    Ok(Json(json!({"ok": true})))
}
