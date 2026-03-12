use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::RwLock;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use agent_sdk::AgentEvent;
use crate::db::{messages, sessions};
use crate::web::sse;
use crate::web::state::{AppState, RunAccumulator};

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

    // Create accumulator for SSE reconnection replay
    let accumulator = Arc::new(RwLock::new(RunAccumulator::default()));

    // Store in maps for status/reconnection
    state.active_runs.insert(run_id.clone(), event_tx.clone());
    state.custom_events.insert(run_id.clone(), custom_tx.clone());
    state.run_sessions.insert(run_id.clone(), session_id.clone());
    state.run_accumulators.insert(run_id.clone(), accumulator.clone());

    // Spawn accumulator task — subscribes to agent events and tracks accumulated state
    let accum_rx = event_tx.subscribe();
    let accum_clone = accumulator.clone();
    tokio::spawn(async move {
        let mut stream = BroadcastStream::new(accum_rx);
        while let Some(Ok(envelope)) = stream.next().await {
            let mut acc = accum_clone.write().await;
            match &envelope.event {
                AgentEvent::TextDelta { delta, .. } => {
                    acc.text.push_str(delta);
                }
                AgentEvent::ToolCallStart { id, name, input, .. } => {
                    // UTF-16 code units to match JavaScript's string.length
                    let split_idx = acc.text.encode_utf16().count();
                    let order = acc.tool_calls.len();
                    acc.tool_calls.push(json!({
                        "id": id,
                        "name": name,
                        "input": serde_json::to_string(input).unwrap_or_default(),
                        "status": "running",
                        "order": order,
                        "contentSplitIndex": split_idx,
                    }));
                }
                AgentEvent::ToolCallEnd { id, result, .. } => {
                    for tc in &mut acc.tool_calls {
                        if tc["id"].as_str() == Some(id) {
                            tc["output"] = json!(result.output);
                            tc["status"] = json!(if result.success { "done" } else { "error" });
                        }
                    }
                }
                AgentEvent::Done { .. } => break,
                _ => {}
            }
        }
    });

    // Spawn the agent loop
    let run_id_clone = run_id.clone();
    let session_id_clone = session_id.clone();
    let state_clone = state.clone();
    let model = payload.model.clone();
    let group = payload.group.clone()
        .unwrap_or_else(|| state.config.main_group.clone());

    let handle = tokio::spawn(async move {
        // Determine soul/memory managers for the active group.
        // If a non-default group is requested, create group-specific managers.
        let (soul, memory) = if group != state_clone.config.main_group {
            let group_soul_dir = state_clone.config.groups_dir.join(&group).join("soul");
            if group_soul_dir.exists() {
                let soul = Arc::new(crate::soul::SoulManager::new(group_soul_dir));
                let memory = Arc::new(crate::memory::MemoryManager::new(soul.clone()));
                (soul, memory)
            } else {
                tracing::warn!(group = %group, "group soul dir not found, falling back to main");
                (state_clone.soul.clone(), state_clone.memory.clone())
            }
        } else {
            (state_clone.soul.clone(), state_clone.memory.clone())
        };

        let ctx = crate::context::ClawContext {
            soul,
            memory,
            db: state_clone.db.clone(),
            scheduler: state_clone.scheduler.clone(),
            notification_tx: state_clone.notification_tx.clone(),
            chat_tx: state_clone.chat_tx.clone(),
            pending_questions: state_clone.pending_questions.clone(),
            session_id: Some(session_id_clone.clone()),
            config: state_clone.config.clone(),
            group: group.clone(),
            custom_event_tx: Some(custom_tx),
        };
        let thread_id = agent_sdk::ThreadId::from_string(session_id_clone.clone());
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
        state_clone.run_accumulators.remove(&run_id_clone);
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

/// GET /api/chat/stream/:run_id — SSE reconnection with accumulated state replay
pub async fn stream_events(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let event_tx = state.active_runs.get(&run_id)
        .ok_or((StatusCode::NOT_FOUND, format!("run {run_id} not found")))?
        .value().clone();

    // Build replay events from accumulated state
    let mut replay_events: Vec<serde_json::Value> = Vec::new();
    if let Some(accum_ref) = state.run_accumulators.get(&run_id) {
        let acc = accum_ref.value().read().await;

        // Replay accumulated text as a single text_delta
        if !acc.text.is_empty() {
            replay_events.push(json!({"type": "text_delta", "text": &acc.text}));
        }

        // Replay tool calls
        for tc in &acc.tool_calls {
            replay_events.push(json!({
                "type": "tool_use_start",
                "id": tc["id"],
                "name": tc["name"],
                "input": tc["input"],
            }));
            // If tool has completed, also emit tool_result
            if tc["status"].as_str() != Some("running") {
                replay_events.push(json!({
                    "type": "tool_result",
                    "id": tc["id"],
                    "output": tc["output"],
                    "is_error": tc["status"] == "error",
                }));
            }
        }
    }

    // Create replay stream
    let replay_stream = futures::stream::iter(replay_events.into_iter().map(|json| {
        let data = serde_json::to_string(&json).unwrap_or_default();
        Ok::<_, Infallible>(Event::default().data(data))
    }));

    // Subscribe to live agent events
    let agent_rx = event_tx.subscribe();
    let agent_stream = BroadcastStream::new(agent_rx).filter_map(|result| match result {
        Ok(envelope) => sse::transform_event(&envelope).map(|json| {
            let data = serde_json::to_string(&json).unwrap_or_default();
            Ok(Event::default().data(data))
        }),
        Err(_) => None,
    });

    // Subscribe to custom events (ask_user, etc.) — create a dummy channel if none exists
    let custom_rx = state.custom_events.get(&run_id)
        .map(|e| e.value().subscribe())
        .unwrap_or_else(|| {
            let (tx, rx) = tokio::sync::broadcast::channel::<serde_json::Value>(1);
            drop(tx); // drop sender immediately — receiver will close
            rx
        });
    let custom_stream = BroadcastStream::new(custom_rx).filter_map(|result| match result {
        Ok(json) => {
            let data = serde_json::to_string(&json).unwrap_or_default();
            Some(Ok(Event::default().data(data)))
        }
        Err(_) => None,
    });

    let live = agent_stream.merge(custom_stream);

    Ok(Sse::new(replay_stream.chain(live)).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    ))
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

    // Send synthetic "done" event through custom_events so the SSE stream
    // gets a clean termination signal before we abort the task.
    if let Some(custom_tx) = state.custom_events.get(&run_id) {
        let done_event = json!({
            "type": "done",
            "result": null,
            "cost_usd": 0,
            "duration_ms": 0,
            "num_turns": 0,
            "input_tokens": 0,
            "output_tokens": 0,
            "cache_read_tokens": 0,
            "cache_creation_tokens": 0,
            "stopped": true
        });
        let _ = custom_tx.send(done_event);
    }

    // Small yield to let the done event propagate through the stream
    tokio::task::yield_now().await;

    // Store partial assistant message from accumulator before aborting
    if let Some(accum_ref) = state.run_accumulators.get(&run_id) {
        let acc = accum_ref.value().read().await;
        if !acc.text.is_empty() {
            if let Some(session_ref) = state.run_sessions.get(&run_id) {
                let session_id = session_ref.value().clone();
                drop(session_ref);
                let metadata = json!({
                    "toolCalls": acc.tool_calls,
                    "resultMeta": {
                        "costUsd": 0,
                        "durationMs": 0,
                        "numTurns": 0,
                        "inputTokens": 0,
                        "outputTokens": 0,
                        "stopped": true,
                    }
                });
                let msg_id = uuid::Uuid::new_v4().to_string();
                let db = state.db.lock().await;
                let _ = messages::store_message(
                    &db, &msg_id, &run_id, "assistant",
                    &acc.text, Some(&session_id),
                    Some(&metadata.to_string()),
                );
            }
        }
    }

    // Abort the agent task
    let handle = state.abort_handles.get(&run_id)
        .ok_or((StatusCode::NOT_FOUND, format!("run {run_id} not found")))?
        .value().clone();

    handle.abort();

    // Clean up all maps
    state.abort_handles.remove(&run_id);
    state.active_runs.remove(&run_id);
    state.custom_events.remove(&run_id);
    state.run_sessions.remove(&run_id);
    state.run_accumulators.remove(&run_id);

    Ok(Json(json!({"ok": true})))
}
