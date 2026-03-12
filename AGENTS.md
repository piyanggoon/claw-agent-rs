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
16. [Authentication](#authentication)
17. [Configuration Reference](#configuration-reference)
18. [Testing](#testing)
19. [Key Design Decisions](#key-design-decisions)
20. [Dependency Map](#dependency-map)

---

## Overview

Claw Agent RS is a self-hosted AI companion backend. It gives an LLM-powered agent persistent
identity, memory, and personality stored in markdown files on disk. The agent reads its "soul"
every session, evolves over time, remembers past conversations, and can schedule autonomous tasks.

**Key traits:**
- Built on [bipa-app/agent-sdk](https://github.com/bipa-app/agent-sdk) (Rust)
- 27 tools: 21 custom native tools + 6 SDK primitives (Read, Write, Edit, Glob, Grep, Bash)
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
 ├── error.rs           Custom error types
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
 │   └── runner.rs      Agent execution orchestration + RunResult
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
     ├── server.rs      Router + axum::serve (public/protected route split)
     ├── state.rs       AppState struct
     ├── sse.rs         AgentEvent → frontend JSON transformer
     ├── middleware.rs   Auth middleware (cookie/bearer token validation)
     └── routes/
         ├── mod.rs         Route module exports
         ├── chat.rs        Chat endpoints (SSE-first)
         ├── sessions.rs    Session endpoints (CRUD + rename)
         ├── history.rs     Message history endpoints
         ├── tasks.rs       Task CRUD + logs + SSE events
         ├── notifications.rs  Notification endpoints
         ├── soul.rs        Soul file endpoints (list, read, write, delete, daily logs)
         ├── groups.rs      Group management endpoints
         ├── auth.rs        Auth endpoints (token generation/verification)
         ├── search.rs      Message search endpoint
         └── files.rs       File serve + upload endpoints
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
    pub auth_enabled: bool,             // Enable/disable auth (AUTH_ENABLED env)
    pub auth_password: Option<String>,  // Password for login (AUTH_PASSWORD env)
    pub auth_secret: String,            // HMAC-SHA256 signing key (AUTH_SECRET or auto-derived)
}
```

**Auth secret derivation:** If `AUTH_SECRET` is not set, it is auto-derived from the password
as `claw-auth-{password}-secret-key`. If auth is enabled but no password is set, the server
should warn at startup.

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
    /// The active group folder name (e.g. "main"). Determines which soul
    /// files, memory, and AGENTS.md are used for this run.
    pub group: String,
    /// Optional broadcast sender for injecting custom SSE events
    /// (e.g., ask_user questions) directly into the frontend's SSE stream.
    /// Set when running via web chat, `None` for background/scheduled tasks.
    pub custom_event_tx: Option<broadcast::Sender<serde_json::Value>>,
}
```

**Event structs:**

```rust
pub struct NotificationEvent { id, title, message, level }
pub struct ChatMessageEvent { session_id, content }
```

### `AppState` (`src/web/state.rs`)

The web server state, shared across all Axum route handlers. Every field is cheaply
cloneable (`Arc` or broadcast handles).

```rust
pub struct AppState {
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub soul: Arc<SoulManager>,
    pub memory: Arc<MemoryManager>,
    pub config: Arc<ClawConfig>,
    pub scheduler: Arc<SchedulerHandle>,
    pub active_runs: Arc<DashMap<String, broadcast::Sender<AgentEventEnvelope>>>,
    pub abort_handles: Arc<DashMap<String, AbortHandle>>,
    pub notification_tx: broadcast::Sender<NotificationEvent>,
    pub chat_tx: broadcast::Sender<ChatMessageEvent>,
    pub pending_questions: Arc<DashMap<String, oneshot::Sender<String>>>,
    /// Maps run_id → session_id for chat status reporting.
    pub run_sessions: Arc<DashMap<String, String>>,
    /// Custom SSE event channels per run (for ask_user, plan_update, etc.).
    pub custom_events: Arc<DashMap<String, broadcast::Sender<serde_json::Value>>>,
    /// Broadcast channel for real-time task lifecycle events (SSE to frontend).
    pub task_events_tx: broadcast::Sender<serde_json::Value>,
    /// Per-run accumulated text + tool calls for SSE reconnection replay.
    pub run_accumulators: Arc<DashMap<String, Arc<RwLock<RunAccumulator>>>>,
}
```

---

## Request Lifecycle

A complete chat request flows through these steps:

### 1. HTTP Request

```
POST /api/chat
{
  "message": "Hello!",
  "webSessionId": "optional-uuid",
  "newSession": false,
  "model": "optional",
  "images": ["optional-base64"],
  "planMode": false,
  "group": "optional"
}
```

### 2. Route Handler (`web/routes/chat.rs::create_chat`)

1. Generate `run_id` (UUID)
2. Determine `session_id` — create new if `newSession=true` or no `webSessionId` provided
3. Ensure web session exists in SQLite
4. Build message text with image references and plan mode prefix
5. Store user message in `messages` table
6. Create two broadcast channels: `event_tx` (agent events, capacity 256) and `custom_tx` (custom events, capacity 64)
7. **Subscribe to both channels BEFORE spawning** (to avoid missing events)
8. Insert into `active_runs`, `custom_events`, `run_sessions`, and `run_accumulators` DashMaps
9. **Spawn accumulator task** — subscribes to agent events, tracks accumulated text + tool calls for SSE reconnection replay
10. **Determine group** — if `payload.group` is set and differs from main, create group-specific `SoulManager` + `MemoryManager`
11. **Spawn tokio task** for agent execution
12. Store `abort_handle` for cancellation
11. Build merged SSE stream: prepend `web_session_id` event, then merge agent + custom streams
12. **Return SSE stream directly** as the response body (not JSON — the response IS the stream)

### 3. Agent Execution (`agent/runner.rs::run_agent`)

Inside the spawned tokio task:

1. **Build system prompt** — assembles AGENTS.md + all soul files + daily logs + datetime
2. **Create LLM provider** — `AnthropicProvider` with the configured model
3. **Create ToolRegistry** — registers all 21 custom tools + 6 SDK primitive tools (via `Adapt<T>` wrapper)
4. **Create ClawHooks** — routes AgentEvents to the broadcast channel
5. **Create SQLite stores** — `SqliteMessageStore` + `SqliteStateStore`
6. **Build AgentConfig** — `{ system_prompt, model, max_turns: 100, streaming: true }`
7. **Build AgentLoop** — via `builder::<ClawContext>().provider().tools().hooks().stores().build_with_stores()`
8. **Run the agent** — `agent.run(thread_id, AgentInput::Text(message), tool_ctx)`
9. **Forward events** — reads from `events.recv()`, accumulates text/metadata, sends to `event_tx`
10. **Wait for completion** — processes `AgentRunState::Done` or `::Error`
11. **Return RunResult** — accumulated text, tool calls, token usage, duration

After run_agent returns, the spawned task:
- Stores the assistant message with metadata (tool calls, cost, tokens) in the DB
- Cleans up: removes from `active_runs`, `custom_events`, `abort_handles`, `run_sessions`

### 4. SSE Streaming (inline in `POST /api/chat` response)

The SSE stream is returned directly from `POST /api/chat`. It consists of:

1. **Initial event**: `{"type": "web_session_id", "web_session_id": "..."}`
2. **Agent event stream**: `AgentEventEnvelope` objects transformed via `sse.rs` into frontend JSON format
3. **Custom event stream**: Raw JSON values from `custom_event_tx` (e.g., `ask_user` questions)
4. The two streams are merged with `tokio_stream::StreamExt::merge()`
5. A keep-alive ping is sent every 15 seconds

**Reconnection:** `GET /api/chat/stream/{run_id}` allows reconnecting to a running agent's event stream.
The `RunAccumulator` replays all accumulated text (as a single `text_delta`) and tool calls
(as `tool_use_start` + `tool_result` pairs) before switching to the live stream.

**Stop behavior:** `POST /api/chat/stop` sends a synthetic `done` event via `custom_events`,
stores the partial assistant message from the accumulator, then aborts the tokio task and cleans up all maps.

### 5. RunResult (`agent/runner.rs`)

The agent runner accumulates data during execution and returns a `RunResult`:

```rust
pub struct RunResult {
    pub accumulated_text: String,       // All text deltas concatenated
    pub tool_calls: Vec<serde_json::Value>, // Tool call records with id, name, input, output, status
    pub total_turns: usize,             // Number of agent turns
    pub input_tokens: u32,              // Total input tokens consumed
    pub output_tokens: u32,             // Total output tokens consumed
    pub duration_ms: u64,               // Agent run duration in milliseconds
}
```

### 6. Complete Flow Diagram

```
Client                          Server                          LLM
  │                               │                              │
  │── POST /api/chat ────────────►│                              │
  │◄── SSE: web_session_id ───────│                              │
  │                               │── build system prompt ──────►│
  │                               │◄── streaming response ───────│
  │◄── SSE: text_delta ───────────│                              │
  │◄── SSE: tool_use_start ───────│                              │
  │                               │── execute tool locally ──────│
  │◄── SSE: tool_result ──────────│                              │
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

21 custom tools are registered in `tools/mod.rs::register_all_tools()`.
6 SDK primitive tools are registered in `agent/runner.rs::run_agent()` via the `Adapt<T>` wrapper:

```rust
// Custom tools (tools/mod.rs)
pub fn register_all_tools(registry: &mut ToolRegistry<ClawContext>) {
    registry.register(soul::SoulReadTool);
    registry.register(soul::SoulUpdateTool);
    // ... all 21 custom tools
    registry.register(utility::CodeExecuteTool);
}

// SDK primitive tools (agent/runner.rs) — bridged from Tool<()> to Tool<ClawContext>
let fs = Arc::new(LocalFileSystem::new("/"));
let capabilities = AgentCapabilities::full_access();
tools
    .register(Adapt(ReadTool::new(Arc::clone(&fs), capabilities.clone())))
    .register(Adapt(WriteTool::new(Arc::clone(&fs), capabilities.clone())))
    .register(Adapt(EditTool::new(Arc::clone(&fs), capabilities.clone())))
    .register(Adapt(GlobTool::new(Arc::clone(&fs), capabilities.clone())))
    .register(Adapt(GrepTool::new(Arc::clone(&fs), capabilities.clone())))
    .register(Adapt(BashTool::new(Arc::clone(&fs), capabilities)));
```

### Tool Tiers

| Tier | Behavior | Tools |
|------|----------|-------|
| `Observe` | Auto-allowed by hooks | 26 tools (20 custom + 6 SDK primitives) |
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
| `ask_user` | `question: string` (required), `options: string[]` (optional) | Ask question + block up to 5 minutes for response. Sends via `custom_event_tx` directly into SSE stream (falls back to `chat_tx` if unavailable). Uses oneshot channel via `pending_questions` DashMap |
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

Built with Axum. Middleware: CORS (permissive) + tracing + auth middleware.

Routes are split into two groups:
- **Public routes** — `/api/health` and `/api/auth/*` — no authentication required
- **Protected routes** — all other `/api/*` routes — require valid token when `auth_enabled = true`

The auth middleware (`src/web/middleware.rs`) is applied as an Axum layer on the protected
router. When auth is disabled (`AUTH_ENABLED=0`), all routes pass through without token checks.

### Chat Endpoints (Protected)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/api/chat` | Send message, returns SSE stream directly. First event: `{type: "web_session_id", ...}`. Body: `{message, webSessionId?, newSession?, model?, images?, planMode?, group?, mode?}` |
| `GET` | `/api/chat/status` | Returns `{running: bool, runs: [{runId, sessionId}]}` |
| `GET` | `/api/chat/stream/{run_id}` | SSE reconnection to a running agent's event stream |
| `POST` | `/api/chat/respond` | Resolve a pending `ask_user` question. Body: `{question_id, response, run_id?}` |
| `POST` | `/api/chat/stop` | Stop a running agent. Body: `{runId?, sessionId?}`. Resolves run by runId, sessionId, or picks the first active run |

### Session Endpoints (Protected)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/sessions` | List all sessions (ordered by last_message_at desc) with message counts |
| `GET` | `/api/sessions/{id}` | Get session detail + all messages |
| `PATCH` | `/api/sessions/{id}` | Rename session. Body: `{title: string}` |
| `DELETE` | `/api/sessions/{id}` | Delete session and its messages |
| `DELETE` | `/api/sessions` | Delete ALL sessions and messages |

### History Endpoints (Protected)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/history` | Get paginated messages. Query: `?session=&limit=&before=&date=&paginate=`. Returns `{messages, hasMore, total}` |
| `DELETE` | `/api/history` | Delete all messages |

### Task Endpoints (Protected)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/tasks` | List all tasks. Query: `?group=` for filtering |
| `POST` | `/api/tasks` | Create task. Body: `{prompt, schedule_type, schedule_value, group_folder?, context_mode?, web_session_id?}` |
| `GET` | `/api/tasks/{id}` | Get single task |
| `PATCH` | `/api/tasks/{id}` | Update task. Body: `{status?}` |
| `DELETE` | `/api/tasks/{id}` | Delete task |
| `POST` | `/api/tasks/{id}/pause` | Set status = "paused" |
| `POST` | `/api/tasks/{id}/resume` | Set status = "active" |
| `POST` | `/api/tasks/{id}/cancel` | Delete task from DB |
| `GET` | `/api/tasks/{id}/logs` | Get run history for a task. Query: `?limit=20` |
| `GET` | `/api/tasks/logs` | Get all task run logs. Query: `?limit=50` |
| `GET` | `/api/tasks/events` | SSE stream for real-time task lifecycle events (task_created, task_updated, task_paused, task_resumed, task_cancelled, task_deleted) |

### Notification Endpoints (Protected)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/notifications` | List notifications. Query: `?unread=bool&limit=N`. Returns `{notifications, unreadCount}` |
| `PATCH` | `/api/notifications/{id}/read` | Mark single notification as read |
| `POST` | `/api/notifications/read-all` | Mark all notifications as read |
| `DELETE` | `/api/notifications/{id}` | Delete notification |

### Soul Endpoints (Protected)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/soul` | List all soul files with metadata (path, name, size, modified_at). Query: `?group=` |
| `GET` | `/api/soul/memory/search` | Search memory. Query: `?q=query&days=7` |
| `GET` | `/api/soul/memory/daily` | List daily log files (filename, date, size, modified_at), sorted by date desc |
| `GET` | `/api/soul/{*filename}` | Read soul file content. Returns `{filename, content, path}` |
| `PUT` | `/api/soul/{*filename}` | Write soul file. Body: `{content: string}` (JSON). Returns `{ok, filename, size, modified_at}` |
| `DELETE` | `/api/soul/{*filename}` | Delete soul file. **Only BOOTSTRAP.md allowed** (403 otherwise) |

### Groups Endpoints (Protected)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/groups` | List all groups (excludes "default"). Returns `{groups: [{name, folder, trigger}]}` |
| `POST` | `/api/groups` | Create group from default template. Body: `{name, folder, trigger?}` |
| `DELETE` | `/api/groups/{folder}` | Delete group. Cannot delete "default" or the main group |

### Auth Endpoints (Public — no auth required)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/auth/status` | Returns `{auth_enabled: bool}`. Indicates whether authentication is active |
| `POST` | `/api/auth/login` | Body: `{password: string}`. Validates password, returns `{ok: true, token: string}` and sets `claw-token` cookie. Token is HMAC-SHA256 signed, valid for 7 days |
| `POST` | `/api/auth/logout` | Clears the `claw-token` cookie. Returns `{ok: true}` |
| `GET` | `/api/auth/verify` | Validates token from `claw-token` cookie or `Authorization: Bearer` header. Returns `{ok: true}` or 401 |

### Search Endpoints (Protected)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/search` | Search messages by content. Query: `?q=`. Returns `{results: [{sessionId, sessionDate, matchCount, preview}]}` |

### File Endpoints (Protected)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/file` | Serve a file. Query: `?path=`. Returns file content with appropriate Content-Type header |
| `POST` | `/api/upload` | Upload file(s) via multipart. Returns `{files: [{filename, path, url}]}` |

### Health (Public — no auth required)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/health` | Returns `"ok"` |

---

## SSE Event Protocol

### Event Transformer (`src/web/sse.rs`)

The `sse.rs` module transforms raw `AgentEventEnvelope` objects from agent-sdk into the
simplified JSON format expected by the SoulClaw frontend. The function `transform_event()`
takes an envelope and returns `Option<Value>` — returning `None` for events that should
be silently skipped.

### Event Type Mapping

| agent-sdk Event | Frontend Type | Key Fields | Description |
|-----------------|---------------|------------|-------------|
| `TextDelta` | `text_delta` | `text` | Token-by-token streaming text |
| `ThinkingDelta` | `thinking` | `text` | Extended thinking / chain-of-thought |
| `ToolCallStart` | `tool_use_start` | `id`, `name`, `input` | Before tool execution begins |
| `ToolCallEnd` | `tool_result` | `id`, `output`, `is_error` | After tool execution completes |
| `ToolProgress` | `tool_progress` | `tool_use_id`, `tool_name`, `parent_tool_use_id`, `elapsed_seconds` | Long-running tool progress updates |
| `SubagentProgress` (not completed) | `sub_tool_use_start` | `id`, `name`, `input`, `parent_tool_use_id` | Sub-agent starts a tool call |
| `SubagentProgress` (completed) | `sub_tool_result` | `id`, `output`, `is_error`, `parent_tool_use_id` | Sub-agent finishes a tool call |
| `Done` | `done` | `result`, `cost_usd`, `duration_ms`, `num_turns`, `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_creation_tokens` | Agent completed successfully |
| `Error` | `error` | `error` | Agent error message |
| `Start`, `Text`, `Thinking`, `TurnComplete`, `ToolRequiresConfirmation`, `Refusal`, `ContextCompacted` | (skipped) | — | Not relevant to frontend, silently dropped |

### Cost Estimation

`sse.rs` includes `estimate_cost(input_tokens, output_tokens)` which calculates an approximate
USD cost using Claude Sonnet pricing ($3/M input, $15/M output). The result is rounded to
6 decimal places.

### Custom Events

In addition to transformed agent events, the SSE stream can contain custom events injected
via `custom_event_tx`. These are passed through as-is without transformation. Current custom
event types:

| Type | Source | Fields |
|------|--------|--------|
| `ask_user` | `AskUserTool` | `question_id`, `question`, `options` |

### Dual Broadcast Channel Architecture

Each chat run creates two separate broadcast channels that are merged into a single SSE stream:

```
                    ┌─────────────────────┐
                    │   Agent Event TX    │  capacity: 256
                    │  (AgentEventEnvelope)│
                    └─────────┬───────────┘
                              │ transform_event()
                              ▼
┌──────────────┐      ┌──────────────┐
│ Custom Event │      │  Merged SSE  │ → Client
│     TX       │─────►│   Stream     │
│ (serde_json) │      └──────────────┘
└──────────────┘
  capacity: 64
```

- **Agent Event TX**: Carries `AgentEventEnvelope` from the agent-sdk. Transformed by `sse.rs` before sending.
- **Custom Event TX**: Carries raw `serde_json::Value`. Passed through directly. Used by `ask_user` and other tools that need to inject events into the SSE stream without going through the agent-sdk event system.

### ask_user Protocol

The `ask_user` tool now sends its question payload via `custom_event_tx` directly into the SSE
stream (if available), falling back to `chat_tx` for background/scheduled tasks:

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
{ "question_id": "uuid", "response": "a.rs" }
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

## Authentication

### Overview

The authentication system protects API routes with token-based auth using HMAC-SHA256 signed
tokens. It is **opt-in** — disabled by default (`AUTH_ENABLED=0`). When enabled, all API
routes except `/api/health` and `/api/auth/*` require a valid token.

### Configuration

```bash
AUTH_ENABLED=1                          # Enable auth (default: 0)
AUTH_PASSWORD=my-secret-password        # Required when auth is enabled
AUTH_SECRET=custom-hmac-secret          # Optional — auto-derived if not set
```

If `AUTH_SECRET` is not set, it is derived as: `claw-auth-{AUTH_PASSWORD}-secret-key`

### Token Format

Tokens use a custom compact format: `{payloadB64url}.{signatureB64url}`

```
Payload (JSON, base64url-encoded):
{
  "exp": 1741234567890    // Expiration as Unix timestamp in milliseconds
}

Signature:
HMAC-SHA256(payloadB64url, auth_secret) → base64url-encoded
```

Tokens are valid for **7 days** from issuance.

### Token Verification

The token is extracted from (checked in order):
1. `claw-token` cookie
2. `Authorization: Bearer {token}` header

Verification steps:
1. Split token at `.` → payload + signature
2. Base64url-decode both parts
3. Recompute HMAC-SHA256 of the payload using `auth_secret`
4. Compare signatures (constant-time)
5. Decode payload JSON, check `exp` > current time in milliseconds
6. If any step fails → 401 Unauthorized

### Middleware (`src/web/middleware.rs`)

The `auth_middleware` function is an Axum middleware applied to the protected router layer.

```rust
pub async fn auth_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response
```

**Behavior:**
- If `config.auth_enabled == false` → pass through (all requests allowed)
- If `config.auth_enabled == true` → extract token from cookie or bearer header, verify, and either allow or return 401

### Route Split (`src/web/server.rs`)

```
Router
├── Public (no middleware)
│   ├── GET  /api/health
│   └── /api/auth/*
│       ├── GET  /api/auth/status
│       ├── POST /api/auth/login
│       ├── POST /api/auth/logout
│       └── GET  /api/auth/verify
│
└── Protected (auth_middleware layer)
    ├── /api/chat/*
    ├── /api/sessions/*
    ├── /api/history/*
    ├── /api/tasks/*
    ├── /api/notifications/*
    ├── /api/soul/*
    ├── /api/groups/*
    ├── /api/search
    ├── /api/file
    └── /api/upload
```

### Login Flow

1. Client sends `POST /api/auth/login` with `{password: "..."}`
2. Server compares password against `config.auth_password`
3. On match: generates token (7-day expiry), sets `claw-token` cookie, returns `{ok: true, token: "..."}`
4. On mismatch: returns 401 `{error: "Invalid password"}`

### Files Modified

| File | Changes |
|------|---------|
| `src/config.rs` | Added `auth_enabled`, `auth_password`, `auth_secret` fields to `ClawConfig` |
| `src/web/middleware.rs` | **New file.** Auth middleware with token verification |
| `src/web/routes/auth.rs` | Replaced stubs with real token generation and verification |
| `src/web/server.rs` | Split router into public and protected route groups |
| `Cargo.toml` | Added `hmac = "0.12"`, `sha2 = "0.10"`, `base64 = "0.22"` |
| `tests/auth_tests.rs` | **New file.** 25 tests covering the auth system |

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
| `AUTH_ENABLED` | `0` | 0/1 | Enable authentication (`1` = enabled, `0` = disabled) |
| `AUTH_PASSWORD` | — | String | Password for login. Required when `AUTH_ENABLED=1` |
| `AUTH_SECRET` | auto-derived | String | HMAC-SHA256 signing key. If not set, derived as `claw-auth-{password}-secret-key` |
| `RUST_LOG` | `claw_agent_rs=info,tower_http=info` | Filter | tracing-subscriber env filter |

---

## Testing

### Test Files

11 test files in `tests/`, 240 total tests:

| File | Count | Description |
|------|-------|-------------|
| `tests/web_api_extended.rs` | 52 | Extended web API tests (groups, history, sessions, notifications, tasks, soul, search, upload) |
| `tests/scheduler_tests.rs` | 27 | Scheduler timing + pagination (ordering, cursor, has_messages_before, session_stats) |
| `tests/auth_tests.rs` | 25 | Token generation/verification, middleware, login/logout, cookie/bearer auth, public vs protected routes |
| `tests/db_tests.rs` | 25 | SQLite CRUD (sessions, messages, tasks, notifications) |
| `tests/sse_tests.rs` | 23 | SSE event transformation (agent events → frontend JSON) |
| `tests/tools_integration.rs` | 22 | Async tool execution for all 21 custom tools + error cases |
| `tests/soul_manager.rs` | 16 | SoulManager I/O (read, write, update_section, delete, daily_logs) |
| `tests/memory_manager.rs` | 14 | MemoryManager (save, recall, forget, daily_log) |
| `tests/web_api.rs` | 10 | Core web API endpoints (health, chat, soul) |
| `tests/prompt_tests.rs` | 7 | System prompt assembly (soul files, AGENTS.md, daily logs, bootstrap) |
| `tests/config_tests.rs` | 3 | ClawConfig loading and defaults |

### Running Tests

```bash
cargo test
```

### Test Infrastructure

- Tests create `ClawContext` instances with temp directories, in-memory SQLite, and broadcast channels
- Tools tests call `tool.execute(&tool_ctx, input)` directly, validating `ToolResult.success` and output content
- `test_15_ask_user` uses a poll loop (100ms x 50 iterations) to handle async oneshot
- Web API tests use `axum::test` or direct handler invocation

---

## Key Design Decisions

### 1. SSE-first chat response

`POST /api/chat` returns the SSE stream directly as its response body, rather than returning
a JSON response with a `run_id` and requiring a separate `GET /stream/{run_id}` call. The
first event in the stream is `{type: "web_session_id", web_session_id: "..."}` so the
frontend knows which session was created/used. This eliminates the race condition between
receiving the run_id and subscribing to events.

### 2. Dual broadcast channels

Each chat run creates two broadcast channels: one for `AgentEventEnvelope` (from agent-sdk)
and one for custom `serde_json::Value` events. The agent event channel is transformed through
`sse.rs` before being sent to the client. The custom event channel passes through raw JSON.
Both are merged into a single SSE stream via `tokio_stream::StreamExt::merge()`.

### 3. RunResult accumulation

The agent runner accumulates text deltas, tool call records, and token usage as it processes
events. This is returned as a `RunResult` struct, allowing the chat handler to persist the
complete assistant response and metadata to the database after the run completes.

### 4. Native async tools (no `#[async_trait]`)

agent-sdk's `Tool` trait uses `impl Future<...> + Send` instead of `#[async_trait]`.
This avoids the boxing overhead of async_trait. Pattern: capture `Arc`-cloned fields
from `ctx.app` before the async block.

### 5. Line-based markdown parsing (no regex lookahead)

Rust's `regex` crate doesn't support lookahead `(?=...)`. The markdown section parser
was rewritten to use line-by-line iteration instead of regex.

### 6. `lib.rs` + `main.rs` pattern

Created `src/lib.rs` with `pub mod` exports so integration tests can `use claw_agent_rs::*`.
Without this, `main.rs`-only modules would be private to the binary crate.

### 7. DashMap for concurrent state

`active_runs`, `abort_handles`, `pending_questions`, `run_sessions`, and `custom_events`
all use `DashMap` for lock-free concurrent access from multiple tokio tasks.

### 8. dotenvy does NOT override

`dotenvy::dotenv()` does not override existing env vars from the parent process.
If `WEB_PORT=3102` is set in the parent shell, `.env`'s `WEB_PORT=3100` is ignored.

### 9. Broadcast channel for SSE

Each agent run creates its own `broadcast::channel(256)`. SSE clients subscribe as
`BroadcastStream` consumers. Lagged events (client fell behind) are silently skipped.

### 10. Poll-based scheduler with Notify

The scheduler uses `tokio::select!` between `sleep(interval)` and `scheduler_handle.notified()`.
This means new tasks are picked up almost instantly (via `notify_new_task()`) while the
poll interval handles edge cases.

### 11. RunAccumulator for SSE reconnection

Each chat run spawns a separate accumulator task that subscribes to the broadcast channel
and tracks accumulated text + tool calls. On SSE reconnection (`GET /api/chat/stream/{run_id}`),
the accumulated state is replayed before switching to the live stream.

### 12. UTF-16 contentSplitIndex

`contentSplitIndex` (used by the frontend to split text around tool calls) must use UTF-16
code units (`str.encode_utf16().count()`) to match JavaScript's `string.length`. Rust's
`String::len()` returns UTF-8 bytes which differs significantly for non-ASCII text
(e.g., Thai "สวัสดี" = 18 bytes but 6 UTF-16 code units).

### 13. Adapt<T> wrapper for SDK primitive tools

SDK primitive tools implement `Tool<()>` but our registry is `ToolRegistry<ClawContext>`.
The `Adapt<T>` wrapper in `runner.rs` bridges the gap by creating a dummy `ToolContext<()>`
before delegating to the inner tool. This avoids modifying the SDK.

### 14. Dynamic group selection per chat request

When `payload.group` differs from `config.main_group`, `create_chat` creates temporary
`SoulManager` and `MemoryManager` instances pointing to that group's soul directory.
These are passed through `ClawContext` so the agent reads the correct soul files,
memory, and AGENTS.md for the selected group.

### 15. Pagination with DESC + reverse

`get_messages_paginated` uses `ORDER BY timestamp DESC LIMIT N` then `messages.reverse()`
to get the most recent N messages in chronological order. The `before` cursor is an ISO
timestamp (not a message ID) to match what the frontend sends.

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
| `hmac` | 0.12 | HMAC-SHA256 token signing (auth) |
| `sha2` | 0.10 | SHA-256 digest for HMAC (auth) |
| `base64` | 0.22 | Base64url encoding/decoding for auth tokens |
| `async-trait` | 0.1 | AgentHooks trait |
| `futures` | 0.3 | Stream utilities |
| `tempfile` | 3 (dev) | Temp dirs for tests |
