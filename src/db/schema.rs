use rusqlite::Connection;

pub fn initialize_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS scheduled_tasks (
            id TEXT PRIMARY KEY,
            group_folder TEXT,
            prompt TEXT,
            schedule_type TEXT,
            schedule_value TEXT,
            context_mode TEXT DEFAULT 'isolated',
            context_session TEXT,
            next_run TEXT,
            last_run TEXT,
            last_result TEXT,
            status TEXT DEFAULT 'active',
            created_at TEXT
        );

        CREATE TABLE IF NOT EXISTS task_run_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT REFERENCES scheduled_tasks(id),
            run_at TEXT,
            duration_ms INTEGER,
            status TEXT,
            result TEXT,
            error TEXT
        );

        CREATE TABLE IF NOT EXISTS web_sessions (
            id TEXT PRIMARY KEY,
            title TEXT,
            summary TEXT,
            created_at TEXT,
            last_message_at TEXT
        );

        CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            thread_id TEXT,
            role TEXT,
            content TEXT,
            timestamp TEXT,
            metadata TEXT,
            web_session_id TEXT REFERENCES web_sessions(id)
        );

        CREATE TABLE IF NOT EXISTS notifications (
            id TEXT PRIMARY KEY,
            task_id TEXT,
            source TEXT DEFAULT 'system',
            title TEXT,
            message TEXT,
            level TEXT DEFAULT 'info',
            read INTEGER DEFAULT 0,
            created_at TEXT
        );

        CREATE TABLE IF NOT EXISTS agent_messages (
            id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS agent_states (
            thread_id TEXT PRIMARY KEY,
            data TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_next_run ON scheduled_tasks(next_run);
        CREATE INDEX IF NOT EXISTS idx_status ON scheduled_tasks(status);
        CREATE INDEX IF NOT EXISTS idx_task_run_logs ON task_run_logs(task_id);
        CREATE INDEX IF NOT EXISTS idx_web_session_id ON messages(web_session_id);
        CREATE INDEX IF NOT EXISTS idx_timestamp ON messages(timestamp);
        CREATE INDEX IF NOT EXISTS idx_notifications_read ON notifications(read);
        CREATE INDEX IF NOT EXISTS idx_notifications_created ON notifications(created_at);
        CREATE INDEX IF NOT EXISTS idx_agent_messages_thread ON agent_messages(thread_id);
        ",
    )?;
    Ok(())
}
