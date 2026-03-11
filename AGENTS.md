# Claw Agent RS — Developer Guide

Comprehensive technical documentation for contributors and maintainers of **claw-agent-rs**,
a personal AI companion backend built in Rust.

---

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Core Components](#core-components)
4. [Request Lifecycle](#request-lifecycle)
5. [System Prompt Assembly](#system-prompt-assembly)
6. [Tools System](#tools-system)
7. [Soul File System](#soul-file-system)
8. [Memory System](#memory-system)
9. [Database Schema](#database-schema)
10. [Task Scheduler](#task-scheduler)
11. [Web API](#web-api)
12. [SSE Event Protocol](#sse-event-protocol)
13. [Multi-Group Architecture](#multi-group-architecture)
14. [LLM Provider System](#llm-provider-system)
15. [Hooks System](#hooks-system)
16. [Configuration Reference](#configuration-reference)
17. [Testing](#testing)
18. [Key Design Decisions](#key-design-decisions)
19. [Dependency Map](#dependency-map)

---

## Overview

Claw Agent RS is a self-hosted AI companion backend. It gives an LLM-powered agent persistent
identity, memory, and personality stored in markdown files on disk. The agent reads its "soul"
every session, evolves over time, remembers past conversations, and can schedule autonomous tasks.

**Key traits:**
- Built on [bipa-app/agent-sdk](https://github.com/bipa-app/agent-sdk) (Rust)
- 21 custom native tools executed in-process (zero IPC)
- Two-layer memory: permanent (MEMORY.md) + daily logs (decay over time)
- Poll-based task scheduler with semaphore-limited concurrency
- Axum HTTP server with SSE streaming
- SQLite for persistence (sessions, messages, tasks, notifications)
- Multi-group support — each group is an independent agent instance

---

## Architecture

```
                    ┌──────────────┐
                    │   Frontend   │   Any HTTP client / Web UI
                    └──────┬───────┘
                           │ HTTP + SSE
                    ┌──────▼───────┐
                    │  Axum Server │   REST API + SSE streaming
                    │  (web/)      │
                    └──────┬───────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
      ┌───────▼──────┐ ┌──▼──────┐ ┌───▼────────┐
      │ Agent Runner │ │Scheduler│ │  SQLite DB  │
      │ (agent/)     │ │(sched/) │ │  (db/)      │
      └───────┬──────┘ └─────────┘ └────────────┘
              │
      ┌───────▼───────┐
      │  agent-sdk    │   LLM provider + tool execution loop
      └───────┬───────┘
              │
   ┌──────────▼──────────────────────────────────────┐
   │              21 Custom Tools (tools/)            │
   │                                                  │
   │  Soul (4)  │ Memory (4)  │ Heartbeat (2)        │
   │  Tasks (5) │ Utility (6)                         │
   └──────────────────────────┬──────────────────────┘
                              │
                       ┌──────▼──────┐
                       │  Soul Files │   groups/{name}/soul/*.md
                       │  (on disk)  │
                       └─────────────┘
```

### Module Dependency Graph

```
main.rs
 ├── config.rs          ClawConfig (env vars → struct)
 ├── context.rs         ClawContext (shared state for tools)
 ├── hooks.rs           ClawHooks (AgentHooks trait impl)
 ├── db/
 │   ├── schema.rs      SQLite CREATE TABLE statements
 │   ├── stores.rs      agent-sdk MessageStore/StateStore trait impls
 │   ├── sessions.rs    Web sessions CRUD
 │   ├── messages.rs    Chat messages CRUD
 │   ├── tasks.rs       Scheduled tasks CRUD
 │   └── notifications.rs  Notifications CRUD
 ├── soul/
 │   ├── manager.rs     SoulManager — soul file I/O
 │   ├── markdown.rs    Markdown section parser/updater
 │   └── prompt.rs      System prompt assembly
 ├── memory/
 │   └── manager.rs     MemoryManager — save/recall/forget/daily_log
 ├── agent/
 │   ├── provider.rs    LLM provider factory (Anthropic)
 │   └── runner.rs      Agent execution orchestration
 ├── scheduler/
 │   └── engine.rs      Poll-based task scheduler
 ├── tools/
 │   ├── mod.rs         register_all_tools() — registry setup
 │   ├── soul.rs        4 soul tools
 │   ├── memory.rs      4 memory tools
 │   ├── heartbeat.rs   2 heartbeat tools
 │   ├── tasks.rs       5 task tools
 │   └── utility.rs     6 utility tools
 └── web/
     ├── server.rs      Router + axum::serve
     ├── state.rs       AppState struct
     └── routes/
         ├── chat.rs       Chat endpoints
         ├── sessions.rs   Session endpoints
         ├── tasks.rs      Task endpoints
         ├── notifications.rs  Notification endpoints
         └── soul.rs       Soul file endpoints
```

---

## Core Components

### `ClawConfig` (`src/config.rs`)

All configuration is loaded from environment variables via `ClawConfig::from_env()`.
The `.env` file is loaded by `dotenvy::dotenv()` at startup.

**Important:** `dotenvy::dotenv()` does NOT override existing environment variables. If a
variable is already set in the parent process, the `.env` value is ignored.

```rust
pub struct ClawConfig {
    pub data_dir: PathBuf,              // SQLite + runtime data
    pub groups_dir: PathBuf,            // groups/ directory root
    pub main_group: String,             // Active group name
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub default_model: String,          // LLM model identifier
    pub web_port: u16,
    pub timezone: String,
    pub scheduler_poll_interval: Duration,
    pub max_concurrent_tasks: usize,
    pub agent_timeout: Duration,
}
```

Helper method: `config.soul_dir()` → `{groups_dir}/{main_group}/soul`

### `ClawContext` (`src/context.rs`)

The shared application context passed to every tool invocation via `ToolContext<ClawContext>`.
All fields are behind `Arc` or are `Clone`-friendly, so cloning is cheap.

```rust
pub struct ClawContext {
    pub soul: Arc<SoulManager>,
    pub memory: Arc<MemoryManager>,
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub scheduler: Arc<SchedulerHandle>,
    pub notification_tx: broadcast::Sender<NotificationEvent>,
    pub chat_tx: broadcast::Sender<ChatMessageEvent>,
    pub pending_questions: Arc<DashMap<String, oneshot::Sender<String>>>,
    pub session_id: Option<String>,
    pub config: Arc<ClawConfig>,
}
```

**Event structs:**

```rust
pub struct NotificationEvent { id, title, message, level }
pub struct ChatMessageEvent { session_id, content }
```

### `AppState` (`src/web/state.rs`)

The web server state, similar to `ClawContext` but includes web-specific fields:

- `active_runs: Arc<DashMap<String, broadcast::Sender<AgentEventEnvelope>>>`
- `abort_handles: Arc<DashMap<String, AbortHandle>>`

---

## Request Lifecycle

A complete chat request flows through these steps:

### 1. HTTP Request

```
POST /api/chat
{ "message": "Hello!", "session_id": "optional-uuid", "model": "optional" }
```

### 2. Route Handler (`web/routes/chat.rs::create_chat`)

1. Generate `run_id` (UUID)
2. Resolve or create `session_id`
3. Ensure web session exists in SQLite
4. Store user message in `messages` table
5. Create a `broadcast::channel(256)` for this run
6. Insert `(run_id, tx)` into `active_runs` DashMap
7. **Spawn tokio task** for agent execution
8. Store `abort_handle` for cancellation
9. Return `201 Created` with `{ run_id, session_id }` immediately

### 3. Agent Execution (`agent/runner.rs::run_agent`)

Inside the spawned tokio task:

1. **Build system prompt** — assembles AGENTS.md + all soul files + daily logs + datetime
2. **Create LLM provider** — `AnthropicProvider` with the configured model
3. **Create ToolRegistry** — registers all 21 tools
4. **Create ClawHooks** — routes AgentEvents to the broadcast channel
5. **Create SQLite stores** — `SqliteMessageStore` + `SqliteStateStore`
6. **Build AgentConfig** — `{ system_prompt, model, max_turns: 100, streaming: true }`
7. **Build AgentLoop** — via `builder::<ClawContext>().provider().tools().hooks().stores().build_with_stores()`
8. **Run the agent** — `agent.run(thread_id, AgentInput::Text(message), tool_ctx)`
9. **Forward events** — reads from `events.recv()` and sends to `event_tx`
10. **Wait for completion** — processes `AgentRunState::Done` or `::Error`
11. **Cleanup** — removes from `active_runs` and `abort_handles`

### 4. SSE Streaming (`web/routes/chat.rs::stream_events`)

```
GET /api/chat/stream/{run_id}
```

1. Look up `run_id` in `active_runs` DashMap
2. Create a new `BroadcastStream` subscriber
3. Return Axum `Sse<Stream>` that serializes each `AgentEventEnvelope` as JSON
4. Stream ends when the broadcast sender is dropped (agent finished)

### 5. Complete Flow Diagram

```
Client                          Server                          LLM
  │                               │                              │
  │── POST /api/chat ────────────►│                              │
  │◄── 201 {run_id, session_id} ──│                              │
  │                               │── build system prompt ──────►│
  │── GET /stream/{run_id} ──────►│                              │
  │                               │◄── streaming response ───────│
  │◄── SSE: text_delta ───────────│                              │
  │◄── SSE: tool_call_start ──────│                              │
  │                               │── execute tool locally ──────│
  │◄── SSE: tool_call_end ────────│                              │
  │                               │── tool result → LLM ────────►│
  │◄── SSE: text_delta ───────────│◄── continue response ────────│
  │◄── SSE: done ─────────────────│                              │
  │                               │                              │
```

---

## System Prompt Assembly

The system prompt is built by `soul/prompt.rs::build_system_prompt()` and injected into
every agent run.

### Assembly Order

```
1. AGENTS.md             ← Group-level instructions (from groups/{name}/AGENTS.md)
2. SOUL.md               ← Personality & values
3. IDENTITY.md           ← Name, creature type, vibe
4. USER.md               ← User profile & preferences
5. MEMORY.md             ← Long-term memory (sections)
6. HEARTBEAT.md          ← Working memory / current state
7. TOOLS.md              ← Environment notes
8. BOOTSTRAP.md          ← Only if it exists (first run)
9. Recent Daily Logs     ← Last 3 days from soul/memory/YYYY-MM-DD.md
10. Current datetime     ← Timestamp with timezone
```

### Wrapping Format

Each section is wrapped in HTML comment markers:

```html
<!-- SOUL.md -->
(content of SOUL.md)
<!-- /SOUL.md -->
```

This allows the LLM to identify which file each section comes from.

### File Loading

- `AGENTS.md` is read from `{groups_dir}/{main_group}/AGENTS.md` (not from soul/)
- All soul files are read via `SoulManager::read(filename)`
- Missing files are silently skipped (logged at debug level)
- Daily logs are loaded via `MemoryManager::get_recent_daily_logs(3)`

---

## Tools System

### Tool Trait (agent-sdk)

Every tool implements the `Tool<ClawContext>` trait from agent-sdk:

```rust
impl Tool<ClawContext> for MyTool {
    type Name = DynamicToolName;

    fn name(&self) -> DynamicToolName { DynamicToolName::new("my_tool") }
    fn display_name(&self) -> &'static str { "My Tool" }
    fn description(&self) -> &'static str { "What this tool does" }
    fn input_schema(&self) -> Value { json!({...}) }
    fn tier(&self) -> ToolTier { ToolTier::Observe }

    fn execute(
        &self,
        ctx: &ToolContext<ClawContext>,
        input: Value,
    ) -> impl Future<Output = anyhow::Result<ToolResult>> + Send {
        // Access app context via ctx.app
        async move {
            Ok(ToolResult::success("result"))
        }
    }
}
```

**Important notes:**
- The `execute` method uses native async (NOT `#[async_trait]`). The return type is
  `impl Future<Output = ...> + Send`.
- All tools use `DynamicToolName` (not static name types).
- Context is accessed via `ctx.app` which is the `ClawContext`.
- `input` is a `serde_json::Value` matching the `input_schema()`.
- Tools should return `ToolResult::success(...)` or `ToolResult::error(...)`.

### Tool Registration

All 21 tools are registered in `tools/mod.rs::register_all_tools()`:

```rust
pub fn register_all_tools(registry: &mut ToolRegistry<ClawContext>) {
    registry.register(soul::SoulReadTool);
    registry.register(soul::SoulUpdateTool);
    // ... all 21 tools
    registry.register(utility::CodeExecuteTool);
}
```

### Tool Tiers

| Tier | Behavior | Tools |
|------|----------|-------|
| `Observe` | Auto-allowed by hooks | 20 tools |
| `Confirm` | Can require user confirmation | `code_execute` |

Currently, `ClawHooks::pre_tool_use()` returns `ToolDecision::Allow` for all tiers
(including Confirm). This can be changed to route Confirm-tier tools to user approval.

### Complete Tool Reference

#### Soul Tools (4)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `soul_read` | `filename: string` (required) | Read any soul file. Supports paths like `"memory/2026-03-03.md"` |
| `soul_update` | `filename: string`, `content: string` (both required) | Overwrite entire soul file |
| `soul_update_section` | `filename: string`, `heading: string`, `content: string` (all required) | Update a `## heading` section. If section doesn't exist, appends it |
| `soul_delete` | `filename: string` (required) | Delete a soul file. **Only BOOTSTRAP.md is allowed** (safety constraint) |

#### Memory Tools (4)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory_save` | `section: string` (required), `content: string` (required), `action: "append"\|"replace"` (default: "append") | Save to MEMORY.md. Section examples: "Facts", "Preferences", "Instructions", "Insights", "Context" |
| `memory_daily_log` | `content: string` (required), `category: "event"\|"observation"\|"decision"\|"interaction"\|"reflection"` (optional) | Append timestamped entry to `soul/memory/YYYY-MM-DD.md` |
| `memory_recall` | `query: string` (optional), `days: number` (default: 7) | Search MEMORY.md + daily logs. No query = return all content |
| `memory_forget` | `section: string` (required), `entry: string` (required) | Remove matching lines from section in MEMORY.md. Case-insensitive substring match |

#### Heartbeat Tools (2)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `heartbeat_read` | (none) | Read HEARTBEAT.md |
| `heartbeat_update` | `content: string` (required) | Overwrite entire HEARTBEAT.md |

#### Task Tools (5)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `schedule_task` | `prompt: string`, `schedule_type: "cron"\|"interval"\|"once"\|"delay"`, `schedule_value: string` (all required), `context_mode: "group"\|"isolated"` (default: "group") | Create scheduled task. `cron`: cron expr. `interval`: ms between runs. `once`: ISO datetime. `delay`: ms from now (one-shot) |
| `list_tasks` | (none) | List all scheduled tasks as JSON |
| `pause_task` | `task_id: string` (required) | Pause a task (won't run until resumed) |
| `resume_task` | `task_id: string` (required) | Resume a paused task |
| `cancel_task` | `task_id: string` (required) | Delete a task permanently |

#### Utility Tools (6)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `send_notification` | `title: string`, `message: string` (required), `level: "info"\|"success"\|"warning"\|"error"` (default: "info") | Persist notification to DB + broadcast via SSE |
| `send_chat_message` | `content: string` (required), `web_session_id: string` (optional) | Send markdown message to chat UI |
| `ask_user` | `question: string` (required), `options: string[]` (optional) | Ask question + block up to 5 minutes for response. Uses oneshot channel via `pending_questions` DashMap |
| `run_background` | `prompt: string` (required) | Create immediate task (delay=0, isolated) via scheduler |
| `web_fetch` | `url: string` (required), `selector: string` (optional, reserved), `max_length: number` (default: 50000) | HTTP GET with 30s timeout. HTML → plain text via html2text. Truncates at max_length |
| `code_execute` | `language: "javascript"\|"python"\|"bash"` (required), `code: string` (required), `timeout: number` (default: 10000, max: 30000) | Subprocess execution. Returns stdout + stderr. Tier: Confirm |

---

## Soul File System

### SoulManager (`src/soul/manager.rs`)

Manages all soul file I/O. Root directory: `groups/{group}/soul/`

**Core methods:**

| Method | Behavior |
|--------|----------|
| `read(filename)` | `tokio::fs::read_to_string(soul_dir/filename)` |
| `write(filename, content)` | `tokio::fs::write()`. Creates parent dirs as needed |
| `update_section(filename, heading, content)` | Read → parse markdown → replace/append section → write |
| `delete(filename)` | Only allows BOOTSTRAP.md. `tokio::fs::remove_file()` |
| `exists(filename)` | Sync check: `path.exists()` |
| `list_daily_logs(days)` | Reads `soul/memory/YYYY-MM-DD.md` files, sorted by date desc, last N days |

### Markdown Section Parser (`src/soul/markdown.rs`)

**Critical:** Uses **line-based parsing** (not regex). Rust's `regex` crate does not
support lookahead `(?=...)`, which was needed for the original approach.

```rust
pub fn update_section(content: &str, heading: &str, new_content: &str) -> String
```

Algorithm:
1. Split content into lines
2. Find line matching `## {heading}` (exact match)
3. Find end of section (next `## ` heading or EOF)
4. If heading found: replace lines between start and end with new content
5. If heading NOT found: append `## {heading}\n{content}` at end
6. Rebuild string from lines

**Unit tests:** 4 tests in the module covering replace-existing, append-new, empty-file,
and preserve-other-sections.

### Soul File Inventory

| File | Owner | Read by prompt | Updated by |
|------|-------|----------------|------------|
| `SOUL.md` | Agent | Yes (every session) | `soul_update`, `soul_update_section` |
| `IDENTITY.md` | Agent | Yes | `soul_update`, `soul_update_section` |
| `USER.md` | Agent | Yes | `soul_update`, `soul_update_section` |
| `MEMORY.md` | Agent | Yes | `memory_save`, `memory_forget`, `soul_update_section` |
| `HEARTBEAT.md` | Agent | Yes | `heartbeat_update`, `soul_update_section` |
| `TOOLS.md` | Agent | Yes | `soul_update`, `soul_update_section` |
| `BOOTSTRAP.md` | System | Yes (only if exists) | `soul_delete` (delete only) |
| `memory/YYYY-MM-DD.md` | Agent | Last 3 days | `memory_daily_log` |

---

## Memory System

### MemoryManager (`src/memory/manager.rs`)

Higher-level memory operations built on top of `SoulManager`.

#### `save(section, content, action)`

- `action = "append"`: reads existing section content, appends new content, writes back
- `action = "replace"`: delegates directly to `soul.update_section()`

#### `daily_log(content, category?)`

- Generates entry: `- **HH:MM** [category] content`
- Writes to `soul/memory/{YYYY-MM-DD}.md`
- Creates file with header if it doesn't exist: `# Daily Log — YYYY-MM-DD`
- Uses `chrono::Local` for timestamps

#### `recall(query?, days)`

1. Search `MEMORY.md` — case-insensitive line-by-line substring match
2. Search daily logs for last N days — same matching
3. Return results grouped by source file with `=== filename ===` headers
4. If no query, returns ALL content

#### `forget(section, entry)`

1. Read `MEMORY.md`
2. Walk lines, track which `## section` we're in
3. If in target section and line contains entry (case-insensitive), mark for removal
4. Remove marked lines (in reverse to preserve indices)
5. Write updated content

### Two-Layer Memory Architecture

```
Layer 1: MEMORY.md (Permanent)
├── ## Facts
├── ## Preferences
├── ## Instructions
├── ## Insights
└── ## Context

Layer 2: Daily Logs (Ephemeral)
├── soul/memory/2026-03-10.md   ← in context (last 3 days)
├── soul/memory/2026-03-11.md   ← in context
├── soul/memory/2026-03-12.md   ← in context (today)
├── soul/memory/2026-03-09.md   ← NOT in context (searchable via recall)
└── soul/memory/2026-03-08.md   ← NOT in context (searchable via recall)
```

---

## Database Schema

SQLite with WAL mode. Database path: `{data_dir}/claw.db`

### Tables

#### `scheduled_tasks`
```sql
CREATE TABLE scheduled_tasks (
    id TEXT PRIMARY KEY,
    group_folder TEXT,
    prompt TEXT,
    schedule_type TEXT,           -- "cron" | "interval" | "once" | "delay"
    schedule_value TEXT,          -- cron expression | milliseconds | ISO datetime
    context_mode TEXT DEFAULT 'isolated',  -- "group" | "isolated"
    context_session TEXT,         -- session_id for group context
    next_run TEXT,                -- RFC3339 datetime (NULL = no more runs)
    last_run TEXT,
    last_result TEXT,
    status TEXT DEFAULT 'active', -- "active" | "paused" | "running"
    created_at TEXT
);
```

#### `task_run_logs`
```sql
CREATE TABLE task_run_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT REFERENCES scheduled_tasks(id),
    run_at TEXT,
    duration_ms INTEGER,
    status TEXT,                  -- "success" | "error"
    result TEXT,
    error TEXT
);
```

#### `web_sessions`
```sql
CREATE TABLE web_sessions (
    id TEXT PRIMARY KEY,
    title TEXT,
    summary TEXT,
    created_at TEXT,
    last_message_at TEXT
);
```

#### `messages`
```sql
CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    thread_id TEXT,
    role TEXT,                    -- "user" | "assistant"
    content TEXT,
    timestamp TEXT,
    metadata TEXT,
    web_session_id TEXT REFERENCES web_sessions(id)
);
```

#### `notifications`
```sql
CREATE TABLE notifications (
    id TEXT PRIMARY KEY,
    task_id TEXT,
    source TEXT DEFAULT 'system', -- "system" | "agent"
    title TEXT,
    message TEXT,
    level TEXT DEFAULT 'info',    -- "info" | "success" | "warning" | "error"
    read INTEGER DEFAULT 0,
    created_at TEXT
);
```

#### `agent_messages` (agent-sdk MessageStore)
```sql
CREATE TABLE agent_messages (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    data TEXT NOT NULL            -- Serialized agent-sdk message
);
```

#### `agent_states` (agent-sdk StateStore)
```sql
CREATE TABLE agent_states (
    thread_id TEXT PRIMARY KEY,
    data TEXT NOT NULL,           -- Serialized agent state
    updated_at TEXT NOT NULL
);
```

### Indexes

```sql
idx_next_run              ON scheduled_tasks(next_run)
idx_status                ON scheduled_tasks(status)
idx_task_run_logs         ON task_run_logs(task_id)
idx_web_session_id        ON messages(web_session_id)
idx_timestamp             ON messages(timestamp)
idx_notifications_read    ON notifications(read)
idx_notifications_created ON notifications(created_at)
idx_agent_messages_thread ON agent_messages(thread_id)
```

---

## Task Scheduler

### Architecture (`src/scheduler/engine.rs`)

The scheduler is a **poll-based** loop running in a dedicated tokio task.

```
┌─────────────────────────────────────────────┐
│  TaskScheduler::start()                      │
│                                              │
│  loop {                                      │
│    poll_and_execute()                        │
│    select! {                                 │
│      sleep(poll_interval)  ─── timeout ───►  │
│      scheduler_handle.notified() ─── wake ►  │
│    }                                         │
│  }                                           │
└─────────────────────────────────────────────┘
```

### SchedulerHandle

A lightweight handle shared with tools via `ClawContext.scheduler`:

```rust
pub struct SchedulerHandle {
    notify: Notify,
}
```

When `schedule_task` or `run_background` tools create a new task, they call
`scheduler.notify_new_task()` to wake the scheduler immediately instead of waiting
for the next poll interval.

### Execution Flow

1. **Poll:** Query `scheduled_tasks` where `next_run <= NOW` and `status = 'active'`
2. **Semaphore:** Acquire permit from `Semaphore::new(max_concurrent_tasks)` (default: 3)
3. **Mark running:** Set `status = 'running'` in DB
4. **Build context:** Create a fresh `ClawContext` with `pending_questions = DashMap::new()`
5. **Determine thread_id:**
   - `context_mode = "group"` → use task's `context_session` or new ThreadId
   - `context_mode = "isolated"` → always new ThreadId
6. **Run agent:** `run_agent(ctx, thread_id, "[SCHEDULED TASK]\n\n{prompt}", None, event_tx)`
7. **Post-run:** Calculate next_run, log result, update task status

### Next-Run Calculation

| Type | After Run |
|------|-----------|
| `cron` | Next occurrence from cron expression |
| `interval` | `NOW + interval_ms` |
| `once` | `NULL` (no more runs) |
| `delay` | `NULL` (no more runs) |

---

## Web API

### Router (`src/web/server.rs`)

Built with Axum. Middleware: CORS (permissive) + tracing.

### Chat Endpoints

#### `POST /api/chat`
```json
// Request
{ "message": "Hello!", "session_id": "uuid (optional)", "model": "string (optional)" }
// Response (201)
{ "run_id": "uuid", "session_id": "uuid" }
```

Spawns agent in background task. Returns immediately for SSE subscription.

#### `GET /api/chat/stream/{run_id}`
SSE stream. Returns `AgentEventEnvelope` JSON objects. Stream ends when agent finishes.

#### `POST /api/chat/respond`
```json
{ "question_id": "uuid", "answer": "user's response" }
```
Resolves a pending `ask_user` question. Sends answer through oneshot channel.

#### `POST /api/chat/stop/{run_id}`
Aborts the running agent by calling `abort_handle.abort()`.

### Session Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /api/sessions` | List all sessions (ordered by last_message_at desc) |
| `GET /api/sessions/{id}` | Get session detail + messages |
| `DELETE /api/sessions/{id}` | Delete session and its messages |

### Task Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /api/tasks` | List all scheduled tasks |
| `POST /api/tasks/{id}/pause` | Set status = "paused" |
| `POST /api/tasks/{id}/resume` | Set status = "active" |
| `POST /api/tasks/{id}/cancel` | Delete task from DB |
| `GET /api/tasks/{id}/logs` | Get run history for a task |

### Notification Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /api/notifications` | List all (ordered by created_at desc) |
| `POST /api/notifications/{id}/read` | Set read = 1 |
| `POST /api/notifications/read-all` | Set read = 1 for all |
| `DELETE /api/notifications/{id}` | Delete notification |

### Soul Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /api/soul/{*filename}` | Read soul file (e.g. `/api/soul/MEMORY.md`) |
| `PUT /api/soul/{*filename}` | Write soul file |
| `GET /api/soul/memory/search?q=query&days=7` | Search memory |

### Health

| Endpoint | Description |
|----------|-------------|
| `GET /api/health` | Returns `"ok"` |

---

## SSE Event Protocol

Events from `/api/chat/stream/{run_id}` are `AgentEventEnvelope` objects serialized as JSON.

### Event Types

| Type | Fields | When |
|------|--------|------|
| `start` | `thread_id`, `turn` | Agent begins a new turn |
| `tool_call_start` | `name`, `input`, `tier` | Before tool execution |
| `tool_call_end` | `name`, `result: {success, output}` | After tool execution |
| `text_delta` | `delta` | Token-by-token streaming |
| `text` | `text` | Full text message |
| `turn_complete` | `turn`, `usage: {input_tokens, output_tokens}` | Turn finished |
| `done` | `total_turns`, `total_usage`, `duration` | Agent completed |
| `error` | `message` | Agent error |

### ask_user Protocol

The `ask_user` tool sends a special chat message via `chat_tx`:

```json
{
  "type": "ask_user",
  "question_id": "uuid",
  "question": "Which file?",
  "options": ["a.rs", "b.rs"]
}
```

The frontend should display this and POST the answer:

```
POST /api/chat/respond
{ "question_id": "uuid", "answer": "a.rs" }
```

This resolves the oneshot channel in `pending_questions`, unblocking the tool.

---

## Multi-Group Architecture

### Directory Layout

```
groups/
├── default/              # Template — git-tracked
│   ├── AGENTS.md         # System instructions for the agent
│   └── soul/
│       ├── SOUL.md
│       ├── IDENTITY.md
│       ├── USER.md
│       ├── MEMORY.md
│       ├── HEARTBEAT.md
│       ├── TOOLS.md
│       └── BOOTSTRAP.md
├── main/                 # Runtime — git-ignored, auto-created
│   ├── AGENTS.md
│   └── soul/
│       ├── (copies from default)
│       └── memory/       # Daily logs
└── {any-name}/           # Additional groups
```

### Auto-Creation Logic (`src/main.rs`)

At startup, if `groups/{main_group}/soul/` doesn't exist:

1. Verify `groups/default/soul/` exists
2. Create `groups/{main_group}/soul/`
3. Copy all files from `groups/default/soul/` → `groups/{main_group}/soul/`
4. Copy all files from `groups/default/` → `groups/{main_group}/` (AGENTS.md, etc.)
5. Create `groups/{main_group}/soul/memory/` directory

### Usage

```bash
# Default group
cargo run                           # Uses MAIN_GROUP=main

# Custom group
MAIN_GROUP=work cargo run           # Creates groups/work/ from default
MAIN_GROUP=personal cargo run       # Creates groups/personal/ from default
```

Each group is completely independent — separate soul files, separate memories,
separate AGENTS.md instructions.

---

## LLM Provider System

### Provider Factory (`src/agent/provider.rs`)

Currently only Anthropic is implemented. The factory maps model names to provider methods:

```rust
pub fn create_provider(model: &str, config: &ClawConfig) -> AnthropicProvider {
    match model {
        "claude-haiku" | "haiku" => AnthropicProvider::haiku(api_key),
        "claude-opus" | "opus" => AnthropicProvider::opus(api_key),
        "claude-sonnet-4-5" | "sonnet-4-5" => AnthropicProvider::sonnet_45(api_key),
        _ => AnthropicProvider::new(api_key, model.to_string()),
    }
}
```

### Per-Request Model Selection

The `POST /api/chat` endpoint accepts an optional `model` field. If provided, it
overrides `config.default_model` for that request only.

### Future Multi-Provider

The architecture is ready for OpenAI and Google providers. Config already supports
`openai_api_key` and `google_api_key`. To add a new provider:

1. Add routing logic in `create_provider()` based on model name prefix
2. Use the corresponding agent-sdk provider (e.g. `OpenAiProvider`)
3. Return `Box<dyn LlmProvider>` instead of `AnthropicProvider`

---

## Hooks System

### ClawHooks (`src/hooks.rs`)

Implements `AgentHooks` trait (uses `#[async_trait]`):

| Hook | Behavior |
|------|----------|
| `pre_tool_use(name, input, tier)` | Always returns `ToolDecision::Allow`. Future: route Confirm tier to user |
| `post_tool_use(name, result)` | Debug logging only |
| `on_event(event)` | Logs Done (with token usage) and Error events |
| `on_error(error)` | Logs error, returns `true` (continue execution) |
| `on_context_compact(messages)` | Returns `None` (use default compaction) |

---

## Configuration Reference

| Variable | Default | Type | Description |
|----------|---------|------|-------------|
| `DATA_DIR` | `./data` | Path | SQLite database directory |
| `GROUPS_DIR` | `./groups` | Path | Groups directory root |
| `MAIN_GROUP` | `main` | String | Active group name |
| `ANTHROPIC_API_KEY` | — | String | **Required.** Anthropic API key |
| `OPENAI_API_KEY` | — | String | OpenAI API key (future) |
| `GOOGLE_API_KEY` | — | String | Google API key (future) |
| `DEFAULT_MODEL` | `claude-sonnet-4-6` | String | Default LLM model name |
| `WEB_PORT` | `3100` | u16 | HTTP server port |
| `TIMEZONE` | `Asia/Bangkok` | String | Display timezone for timestamps |
| `SCHEDULER_POLL_INTERVAL` | `15` | Seconds | How often the scheduler polls for due tasks |
| `MAX_CONCURRENT_TASKS` | `3` | usize | Maximum parallel task executions |
| `AGENT_TIMEOUT` | `300` | Seconds | Agent execution timeout |
| `RUST_LOG` | `claw_agent_rs=info,tower_http=info` | Filter | tracing-subscriber env filter |

---

## Testing

### Rust Integration Tests (`tests/tools_integration.rs`)

22 async tests that exercise all 21 tools (+ error cases) directly:

- Creates a `ClawContext` with temp directories, in-memory SQLite, and broadcast channels
- Calls `tool.execute(&tool_ctx, input)` directly
- Validates `ToolResult.success` and output content
- `test_15_ask_user` uses a poll loop (100ms × 50 iterations) to handle async oneshot

```bash
cargo test
```

---

## Key Design Decisions

### 1. Native async tools (no `#[async_trait]`)

agent-sdk's `Tool` trait uses `impl Future<...> + Send` instead of `#[async_trait]`.
This avoids the boxing overhead of async_trait. Pattern: capture `Arc`-cloned fields
from `ctx.app` before the async block.

### 2. Line-based markdown parsing (no regex lookahead)

Rust's `regex` crate doesn't support lookahead `(?=...)`. The markdown section parser
was rewritten to use line-by-line iteration instead of regex.

### 3. `lib.rs` + `main.rs` pattern

Created `src/lib.rs` with `pub mod` exports so integration tests can `use claw_agent_rs::*`.
Without this, `main.rs`-only modules would be private to the binary crate.

### 4. DashMap for concurrent state

`active_runs`, `abort_handles`, and `pending_questions` all use `DashMap` for lock-free
concurrent access from multiple tokio tasks.

### 5. dotenvy does NOT override

`dotenvy::dotenv()` does not override existing env vars from the parent process.
If `WEB_PORT=3102` is set in the parent shell, `.env`'s `WEB_PORT=3100` is ignored.

### 6. Broadcast channel for SSE

Each agent run creates its own `broadcast::channel(256)`. SSE clients subscribe as
`BroadcastStream` consumers. Lagged events (client fell behind) are silently skipped.

### 7. Poll-based scheduler with Notify

The scheduler uses `tokio::select!` between `sleep(interval)` and `scheduler_handle.notified()`.
This means new tasks are picked up almost instantly (via `notify_new_task()`) while the
poll interval handles edge cases.

---

## Dependency Map

| Crate | Version | Purpose |
|-------|---------|---------|
| `agent-sdk` | git (main) | Core agent framework — LLM, tools, hooks, stores |
| `tokio` | 1 (full) | Async runtime |
| `tokio-stream` | 0.1 (sync) | BroadcastStream for SSE |
| `axum` | 0.8 (ws) | HTTP server + SSE |
| `axum-extra` | 0.10 | Typed headers |
| `tower` | 0.5 | Middleware tower |
| `tower-http` | 0.6 | CORS, tracing, timeout middleware |
| `rusqlite` | 0.32 (bundled) | SQLite (compiled from source) |
| `serde` | 1 (derive) | Serialization |
| `serde_json` | 1 | JSON handling |
| `cron` | 0.13 | Cron expression parsing |
| `chrono` | 0.4 (serde) | Date/time handling |
| `reqwest` | 0.12 (json, gzip) | HTTP client for web_fetch |
| `html2text` | 0.14 | HTML → plain text conversion |
| `uuid` | 1 (v4, serde) | UUID generation |
| `anyhow` | 1 | Error handling |
| `thiserror` | 2 | Custom error types |
| `regex` | 1 | Pattern matching |
| `glob` | 0.3 | File globbing |
| `dashmap` | 6 | Concurrent HashMap |
| `tracing` | 0.1 | Structured logging |
| `tracing-subscriber` | 0.3 | Log output formatting |
| `dotenvy` | 0.15 | .env file loading |
| `async-trait` | 0.1 | AgentHooks trait |
| `futures` | 0.3 | Stream utilities |
| `tempfile` | 3 (dev) | Temp dirs for tests |
