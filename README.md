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
- **Real-time Streaming** — SSE endpoint streams token-by-token responses and tool call events
- **Task Scheduler** — Cron, interval, once, and delay schedules with poll-based execution
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
# Send a message
curl -X POST http://localhost:3100/api/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello!"}'

# Response: {"run_id": "...", "session_id": "..."}

# Stream events (SSE)
curl -N http://localhost:3100/api/chat/stream/{run_id}
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
│   │   ├── messages.rs       # Chat messages CRUD
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
│       └── routes/
│           ├── chat.rs       # POST /api/chat, GET /api/chat/stream/:id, ...
│           ├── sessions.rs   # GET/DELETE /api/sessions
│           ├── tasks.rs      # GET /api/tasks, POST pause/resume/cancel
│           ├── notifications.rs # GET/POST/DELETE /api/notifications
│           └── soul.rs       # GET/PUT /api/soul/:filename
├── tests/
│   └── tools_integration.rs  # 22 Rust unit tests (all tools)
└── data/                     # Runtime (git-ignored)
    └── claw.db               # SQLite database
```

## API Reference

### Chat

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/chat` | Send message → returns `{run_id, session_id}` |
| `GET` | `/api/chat/stream/{run_id}` | SSE event stream (tool calls, text deltas, done) |
| `POST` | `/api/chat/respond` | Answer an `ask_user` question |
| `POST` | `/api/chat/stop/{run_id}` | Abort a running agent |

### Sessions

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/sessions` | List all sessions |
| `GET` | `/api/sessions/{id}` | Get session by ID |
| `DELETE` | `/api/sessions/{id}` | Delete session |

### Tasks

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/tasks` | List scheduled tasks |
| `POST` | `/api/tasks/{id}/pause` | Pause task |
| `POST` | `/api/tasks/{id}/resume` | Resume task |
| `POST` | `/api/tasks/{id}/cancel` | Cancel & delete task |
| `GET` | `/api/tasks/{id}/logs` | Get task run logs |

### Notifications

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/notifications` | List notifications |
| `POST` | `/api/notifications/{id}/read` | Mark as read |
| `POST` | `/api/notifications/read-all` | Mark all as read |
| `DELETE` | `/api/notifications/{id}` | Delete notification |

### Soul Files

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/soul/{filename}` | Read a soul file |
| `PUT` | `/api/soul/{filename}` | Write a soul file |
| `GET` | `/api/soul/memory/search?q=...` | Search memory |

### Health

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/health` | Returns `"ok"` |

## SSE Event Types

Events streamed from `/api/chat/stream/{run_id}`:

```jsonc
// Agent starts processing a turn
{"type": "start", "thread_id": "...", "turn": 1}

// Tool call begins
{"type": "tool_call_start", "name": "soul_read", "input": {...}, "tier": "Observe"}

// Tool call completes
{"type": "tool_call_end", "name": "soul_read", "result": {"success": true, "output": "..."}}

// Text token streamed
{"type": "text_delta", "delta": "Hello"}

// Full text message
{"type": "text", "text": "Hello! How can I help?"}

// Turn completes
{"type": "turn_complete", "turn": 1, "usage": {"input_tokens": 1000, "output_tokens": 50}}

// Agent finished
{"type": "done", "total_turns": 2, "total_usage": {...}, "duration": {"secs": 5}}
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
| `RUST_LOG` | — | Log filter (e.g. `claw_agent_rs=debug`) |

## Testing

```bash
# Rust unit tests (all 21 tools + error cases) — 22 tests
cargo test
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

To create a new agent, set `MAIN_GROUP=<name>` and restart. The group is auto-provisioned from `groups/default/`.

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

## License

MIT
