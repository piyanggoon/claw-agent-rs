# Claw Agent RS

A personal AI companion backend built in Rust, powered by [agent-sdk](https://github.com/bipa-app/agent-sdk). The agent has persistent identity, memory, and personality stored in markdown files — it learns, remembers, and evolves across conversations.

## Architecture

```
                    ┌──────────────┐
                    │   Frontend   │  (any HTTP client / web UI)
                    └──────┬───────┘
                           │ HTTP + SSE
                    ┌──────▼───────┐
                    │  Axum Server │  REST API + SSE streaming
                    └──────┬───────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
      ┌───────▼──┐  ┌──────▼───┐  ┌────▼──────┐
      │  Agent   │  │ Scheduler│  │  SQLite   │
      │  Runner  │  │  Engine  │  │  Database │
      └───┬──────┘  └──────────┘  └───────────┘
          │
    ┌─────▼─────┐
    │ agent-sdk │  LLM provider + tool execution
    └─────┬─────┘
          │
    ┌─────▼─────────────────────────────────────────┐
    │                 21 Custom Tools                │
    │                                               │
    │  Soul (4)  Memory (4)  Heartbeat (2)          │
    │  Tasks (5)  Utility (6)                       │
    └──────────────────────────┬────────────────────┘
                               │
                        ┌──────▼──────┐
                        │  Soul Files │  groups/{name}/soul/*.md
                        │  (on disk)  │
                        └─────────────┘
```

## Features

- **Persistent Soul** — Identity, personality, and memories live in `.md` files that the agent reads and updates
- **Two-layer Memory** — Core memories (MEMORY.md, never decay) + daily logs (YYYY-MM-DD.md, fade over time)
- **21 Custom Tools** — Soul management, memory, heartbeat, task scheduling, notifications, code execution, web fetch
- **Real-time Streaming** — POST /api/chat returns SSE directly; events stream token-by-token with tool call progress
- **Task Scheduler** — Cron, interval, once, and delay schedules with poll-based execution and real-time SSE events
- **Multi-group Support** — Each group has its own soul files; new groups auto-copy from `groups/default/`
- **Multi-provider Ready** — Architecture supports Anthropic, OpenAI, and Google (Anthropic implemented)
- **SQLite Storage** — Sessions, messages, tasks, notifications all persisted

## Quick Start

### Prerequisites

- Rust 1.85+ (`rustup update`)
- An Anthropic API key

### Setup

```bash
# Clone
git clone <repo-url> && cd claw-agent-rs

# Configure
cp .env.example .env
# Edit .env — set your ANTHROPIC_API_KEY

# Build & run
cargo build --release
./target/release/claw-agent-rs
```

The server starts on port 3100 (configurable via `WEB_PORT`). On first run, `groups/main/` is auto-created from `groups/default/`.

### Chat

```bash
# Send a message (returns SSE stream directly)
curl -N -X POST http://localhost:3100/api/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello!"}'

# Response: SSE event stream
# data: {"type":"web_session_id","web_session_id":"abc-123"}
# data: {"type":"text_delta","text":"Hello"}
# data: {"type":"text_delta","text":"! How"}
# data: {"type":"text_delta","text":" can I help?"}
# data: {"type":"done","result":null,"cost_usd":0.001,"duration_ms":2500,"num_turns":1,"input_tokens":1000,"output_tokens":50,"cache_read_tokens":0,"cache_creation_tokens":0}
```

## Project Structure

```
claw-agent-rs/
├── Cargo.toml
├── .env.example
├── groups/
│   ├── default/              # Template (git-tracked)
│   │   ├── AGENTS.md         # Agent system instructions
│   │   └── soul/
│   │       ├── SOUL.md       # Personality & values
│   │       ├── IDENTITY.md   # Name & role
│   │       ├── USER.md       # User profile
│   │       ├── MEMORY.md     # Long-term memory
│   │       ├── HEARTBEAT.md  # Scheduled tasks
│   │       ├── TOOLS.md      # Environment notes
│   │       └── BOOTSTRAP.md  # First-run onboarding
│   └── main/                 # Runtime (auto-created, git-ignored)
│       ├── AGENTS.md
│       └── soul/
├── src/
│   ├── lib.rs                # Public library exports
│   ├── main.rs               # Entry point
│   ├── config.rs             # Environment configuration
│   ├── context.rs            # ClawContext — shared app state for tools
│   ├── error.rs              # Error types
│   ├── hooks.rs              # AgentHooks implementation
│   ├── agent/
│   │   ├── provider.rs       # LLM provider factory
│   │   └── runner.rs         # Agent execution loop
│   ├── soul/
│   │   ├── manager.rs        # Soul file I/O
│   │   ├── markdown.rs       # Markdown section parser
│   │   └── prompt.rs         # System prompt builder
│   ├── memory/
│   │   └── manager.rs        # Memory save/recall/forget/daily-log
│   ├── db/
│   │   ├── schema.rs         # SQLite tables & indexes
│   │   ├── stores.rs         # agent-sdk MessageStore/StateStore
│   │   ├── sessions.rs       # Web sessions CRUD
│   │   ├── messages.rs       # Chat messages CRUD (with pagination)
│   │   ├── tasks.rs          # Scheduled tasks CRUD
│   │   └── notifications.rs  # Notifications CRUD
│   ├── scheduler/
│   │   └── engine.rs         # Poll-based task scheduler
│   ├── tools/
│   │   ├── mod.rs            # register_all_tools()
│   │   ├── soul.rs           # soul_read, soul_update, soul_update_section, soul_delete
│   │   ├── memory.rs         # memory_save, memory_daily_log, memory_recall, memory_forget
│   │   ├── heartbeat.rs      # heartbeat_read, heartbeat_update
│   │   ├── tasks.rs          # schedule_task, list_tasks, pause_task, resume_task, cancel_task
│   │   └── utility.rs        # send_notification, send_chat_message, ask_user, run_background, web_fetch, code_execute
│   └── web/
│       ├── server.rs         # Axum router & startup
│       ├── state.rs          # AppState
│       ├── sse.rs            # SSE event transformer (agent events → frontend format)
│       ├── middleware.rs     # Auth middleware (token validation, cookie/header extraction)
│       └── routes/
│           ├── chat.rs       # POST /api/chat (SSE), GET status/stream, POST respond/stop
│           ├── sessions.rs   # GET/PATCH/DELETE /api/sessions
│           ├── history.rs    # GET/DELETE /api/history (paginated)
│           ├── tasks.rs      # CRUD + pause/resume/cancel + logs + SSE events
│           ├── notifications.rs # GET/PATCH/DELETE /api/notifications
│           ├── soul.rs       # GET/PUT/DELETE /api/soul, memory search, daily logs
│           ├── groups.rs     # GET/POST/DELETE /api/groups
│           ├── auth.rs       # Auth endpoints (status, login, logout, verify)
│           ├── search.rs     # GET /api/search
│           └── files.rs      # GET /api/file, POST /api/upload
├── tests/
│   ├── tools_integration.rs  # 22 tool tests
│   ├── db_tests.rs           # 25 database tests
│   ├── auth_tests.rs         # 25 auth tests
│   ├── soul_manager.rs       # 16 soul manager tests
│   ├── memory_manager.rs     # 14 memory manager tests
│   ├── web_api.rs            # 10 web API tests
│   ├── prompt_tests.rs       # 7 prompt builder tests
│   └── config_tests.rs       # 3 config tests
└── data/                     # Runtime (git-ignored)
    └── claw.db               # SQLite database
```

## API Reference

### Chat

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/chat` | Send message → returns **SSE stream** directly (not JSON) |
| `GET` | `/api/chat/status` | Check active runs → `{running, runs: [{runId, sessionId}]}` |
| `GET` | `/api/chat/stream/{run_id}` | Reconnect to an existing SSE stream |
| `POST` | `/api/chat/respond` | Answer an `ask_user` question |
| `POST` | `/api/chat/stop` | Abort a running agent (JSON body: `{runId?}` or `{sessionId?}`) |

**POST /api/chat** accepts:
```jsonc
{
  "message": "Hello!",
  "newSession": false,         // Force new session
  "webSessionId": "...",       // Reuse existing session
  "idempotencyKey": "...",     // Dedup key
  "images": ["base64..."],     // Attached images
  "planMode": false,           // Present plan before executing
  "model": "claude-sonnet-4-6",// Override model
  "group": "main",             // Target group
  "mode": "..."                // Custom mode
}
```

The first SSE event is always `{"type": "web_session_id", "web_session_id": "..."}`.

### Sessions

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/sessions` | List all sessions → `{sessions: [...]}` |
| `GET` | `/api/sessions/{id}` | Get session with messages |
| `PATCH` | `/api/sessions/{id}` | Rename session (body: `{title}`) |
| `DELETE` | `/api/sessions/{id}` | Delete session and its messages |
| `DELETE` | `/api/sessions` | Delete all sessions and messages |

### History

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/history` | Paginated message history → `{messages, hasMore, total}` |
| `DELETE` | `/api/history` | Delete all message history |

**GET /api/history** query parameters:

| Param | Description |
|-------|-------------|
| `limit` | Max messages to return (default: 100) |
| `session` | Filter by session ID |
| `before` | Cursor for pagination (message ID) |
| `date` | Alias for session |
| `paginate` | Pagination mode |

### Tasks

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/tasks` | List tasks → `{tasks: [...]}` |
| `POST` | `/api/tasks` | Create a scheduled task → `{task: {...}}` |
| `GET` | `/api/tasks/{id}` | Get task by ID → `{task: {...}}` |
| `PATCH` | `/api/tasks/{id}` | Update task status (body: `{status}`) |
| `DELETE` | `/api/tasks/{id}` | Delete a task |
| `POST` | `/api/tasks/{id}/pause` | Pause task |
| `POST` | `/api/tasks/{id}/resume` | Resume task |
| `POST` | `/api/tasks/{id}/cancel` | Cancel & delete task |
| `GET` | `/api/tasks/{id}/logs` | Get logs for a specific task → `{logs: [...]}` |
| `GET` | `/api/tasks/logs` | Get all task run logs → `{logs: [...]}` |
| `GET` | `/api/tasks/events` | SSE stream for real-time task lifecycle events |

**POST /api/tasks** body:
```jsonc
{
  "prompt": "Check the weather",
  "schedule_type": "cron",       // cron | interval | once | delay
  "schedule_value": "0 9 * * *", // cron expr | ms | ISO datetime | ms delay
  "group_folder": "main",        // Optional, defaults to MAIN_GROUP
  "context_mode": "group",       // group | isolated
  "web_session_id": "..."        // Optional, for session context
}
```

### Notifications

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/notifications` | List notifications → `{notifications: [...], unreadCount}` |
| `PATCH` | `/api/notifications/{id}/read` | Mark notification as read |
| `POST` | `/api/notifications/read-all` | Mark all as read |
| `DELETE` | `/api/notifications/{id}` | Delete notification |

**GET /api/notifications** query parameters: `unread` (bool), `limit` (number).

### Soul Files

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/soul` | List all soul files → `{files: [...]}` |
| `GET` | `/api/soul/{filename}` | Read a soul file (supports nested paths) |
| `PUT` | `/api/soul/{filename}` | Write a soul file (body: `{"content": "..."}`) |
| `DELETE` | `/api/soul/{filename}` | Delete a soul file (BOOTSTRAP.md only) |
| `GET` | `/api/soul/memory/search?q=...&days=7` | Search memory |
| `GET` | `/api/soul/memory/daily` | List daily log files → `{logs: [...]}` |

### Groups

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/groups` | List all groups → `{groups: [...]}` |
| `POST` | `/api/groups` | Create group from default template |
| `DELETE` | `/api/groups/{folder}` | Delete a group (cannot delete default or main) |

**POST /api/groups** body:
```jsonc
{
  "name": "work",
  "folder": "work",
  "trigger": "direct"  // optional
}
```

### Auth

All auth endpoints are **public** (no token required). When `AUTH_ENABLED=1`, all other routes except `/api/health` require authentication via cookie (`claw-token`) or header (`Authorization: Bearer <token>`).

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/api/auth/status` | Public | Check if auth is enabled → `{auth_enabled: bool}` |
| `POST` | `/api/auth/login` | Public | Login with password → `{ok: true, token: "..."}` |
| `POST` | `/api/auth/logout` | Public | Logout (clears cookie) → `{ok: true}` |
| `GET` | `/api/auth/verify?token=...` | Public | Verify token validity → `{ok: true}` |

**POST /api/auth/login** body:
```jsonc
{
  "password": "your-password"
}
```

**Auth behavior by mode:**

| `AUTH_ENABLED` | Login | Protected routes |
|----------------|-------|-----------------|
| `0` (default) | Returns `{ok: true, token: "none"}` | All routes accessible without authentication |
| `1` | Validates password against `AUTH_PASSWORD`, returns HMAC-SHA256 signed token (valid 7 days) | Requires `claw-token` cookie or `Authorization: Bearer` header |

**Token format:** JWT-like with HMAC-SHA256 signature, derived from `AUTH_SECRET` (auto-derived from `AUTH_PASSWORD` if not explicitly set).

### Search

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/search?q=...` | Full-text search across messages → `{results: [...]}` |

Results are grouped by session with preview snippets and match counts.

### Files

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/file?path=...` | Serve a file with correct content-type |
| `POST` | `/api/upload` | Upload files via multipart → `{files: [...]}` |

### Health

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/health` | Returns `"ok"` |

## SSE Event Types

Events streamed from `POST /api/chat` (and reconnectable via `GET /api/chat/stream/{run_id}`):

```jsonc
// Session identifier (always first event)
{"type": "web_session_id", "web_session_id": "abc-123"}

// Text token streamed
{"type": "text_delta", "text": "Hello"}

// Thinking/reasoning token
{"type": "thinking", "text": "Let me consider..."}

// Tool call begins
{"type": "tool_use_start", "id": "tool_1", "name": "soul_read", "input": "{...}"}

// Tool call completes
{"type": "tool_result", "id": "tool_1", "output": "...", "is_error": false}

// Sub-agent tool call begins (nested tool within run_background)
{"type": "sub_tool_use_start", "id": "sub_1", "name": "web_fetch", "input": "", "parent_tool_use_id": "agent_1"}

// Sub-agent tool call completes
{"type": "sub_tool_result", "id": "sub_1", "output": "...", "is_error": false, "parent_tool_use_id": "agent_1"}

// Tool progress heartbeat
{"type": "tool_progress", "tool_use_id": "tool_1", "tool_name": "code_execute", "parent_tool_use_id": null, "elapsed_seconds": 0}

// Agent finished — includes cost and usage stats
{"type": "done", "result": null, "cost_usd": 0.001, "duration_ms": 2500, "num_turns": 1, "input_tokens": 1000, "output_tokens": 50, "cache_read_tokens": 0, "cache_creation_tokens": 0}

// Error occurred
{"type": "error", "error": "rate limit exceeded"}
```

### Task Events (SSE)

Events streamed from `GET /api/tasks/events`:

```jsonc
{"type": "task_created", "task_id": "...", "timestamp": "..."}
{"type": "task_updated", "task_id": "...", "timestamp": "..."}
{"type": "task_paused", "task_id": "...", "timestamp": "..."}
{"type": "task_resumed", "task_id": "...", "timestamp": "..."}
{"type": "task_cancelled", "task_id": "...", "timestamp": "..."}
{"type": "task_deleted", "task_id": "...", "timestamp": "..."}
```

## Custom Tools (21)

| Category | Tool | Description |
|----------|------|-------------|
| **Soul** | `soul_read` | Read any soul file |
| | `soul_update` | Replace entire soul file |
| | `soul_update_section` | Update a `## heading` section |
| | `soul_delete` | Delete BOOTSTRAP.md |
| **Memory** | `memory_save` | Append/replace section in MEMORY.md |
| | `memory_daily_log` | Write timestamped entry to daily log |
| | `memory_recall` | Search MEMORY.md + daily logs |
| | `memory_forget` | Remove entry from MEMORY.md |
| **Heartbeat** | `heartbeat_read` | Read HEARTBEAT.md |
| | `heartbeat_update` | Rewrite HEARTBEAT.md |
| **Tasks** | `schedule_task` | Create cron/interval/once/delay task |
| | `list_tasks` | List all scheduled tasks |
| | `pause_task` | Pause a task |
| | `resume_task` | Resume a paused task |
| | `cancel_task` | Cancel & delete a task |
| **Utility** | `send_notification` | Push notification to web UI |
| | `send_chat_message` | Send message to chat (from background) |
| | `ask_user` | Ask question, wait for response |
| | `run_background` | Spawn background agent task |
| | `web_fetch` | HTTP GET → markdown |
| | `code_execute` | Run bash/python/javascript |

## Configuration

All settings via environment variables (`.env` file supported):

| Variable | Default | Description |
|----------|---------|-------------|
| `ANTHROPIC_API_KEY` | — | **Required.** Anthropic API key |
| `OPENAI_API_KEY` | — | OpenAI API key (future) |
| `GOOGLE_API_KEY` | — | Google API key (future) |
| `DEFAULT_MODEL` | `claude-sonnet-4-6` | Default LLM model |
| `WEB_PORT` | `3100` | HTTP server port |
| `TIMEZONE` | `Asia/Bangkok` | Display timezone |
| `DATA_DIR` | `./data` | SQLite database location |
| `GROUPS_DIR` | `./groups` | Groups directory |
| `MAIN_GROUP` | `main` | Active group name |
| `SCHEDULER_POLL_INTERVAL` | `15` | Scheduler poll interval (seconds) |
| `MAX_CONCURRENT_TASKS` | `3` | Max parallel scheduled tasks |
| `AGENT_TIMEOUT` | `300` | Agent execution timeout (seconds) |
| `AUTH_ENABLED` | `0` | Enable authentication (`0` = off, `1` = on) |
| `AUTH_PASSWORD` | — | Password for login (required when auth enabled) |
| `AUTH_SECRET` | — | HMAC-SHA256 secret for token signing (auto-derived from password if not set) |
| `RUST_LOG` | — | Log filter (e.g. `claw_agent_rs=debug`) |

## Testing

```bash
# Run all tests — 8 test files, 238 tests
cargo test

# Test files:
#   tools_integration.rs  — 22 tool unit tests
#   db_tests.rs           — 25 database CRUD tests
#   auth_tests.rs         — 25 authentication tests (login, token, middleware, cookie/header)
#   soul_manager.rs       — 16 soul file I/O tests
#   memory_manager.rs     — 14 memory manager tests
#   web_api.rs            — 10 web API endpoint tests
#   prompt_tests.rs       — 7 system prompt builder tests
#   config_tests.rs       — 3 configuration tests
#   + 4 inline tests in src/soul/markdown.rs
```

## Multi-Group Support

Each group is an independent agent with its own soul, memory, and identity:

```
groups/
├── default/          # Template — always kept clean
│   ├── AGENTS.md
│   └── soul/*.md
├── main/             # Auto-created on first run
├── work/             # MAIN_GROUP=work → auto-created from default
└── personal/         # MAIN_GROUP=personal → auto-created from default
```

To create a new agent, set `MAIN_GROUP=<name>` and restart. The group is auto-provisioned from `groups/default/`. Groups can also be managed via the `/api/groups` endpoints.

## Response Wrapping

All list endpoints return wrapped responses for consistent frontend consumption:

| Endpoint | Response shape |
|----------|---------------|
| `GET /api/sessions` | `{sessions: [...]}` |
| `GET /api/tasks` | `{tasks: [...]}` |
| `GET /api/notifications` | `{notifications: [...], unreadCount: N}` |
| `GET /api/history` | `{messages: [...], hasMore: bool, total: N}` |
| `GET /api/groups` | `{groups: [...]}` |
| `GET /api/soul` | `{files: [...]}` |
| `GET /api/search` | `{results: [...]}` |

## Dependencies

| Crate | Purpose |
|-------|---------|
| [agent-sdk](https://github.com/bipa-app/agent-sdk) | Core agent framework (LLM, tools, hooks, stores) |
| tokio | Async runtime |
| axum | HTTP server |
| rusqlite | SQLite (bundled) |
| reqwest | HTTP client (web_fetch) |
| chrono + cron | Time handling + cron parsing |
| dashmap | Concurrent HashMap |
| tracing | Structured logging |
| uuid | ID generation |
| futures + tokio-stream | Stream combinators for SSE |

## License

MIT
