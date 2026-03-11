use agent_sdk::llm::Message;
use agent_sdk::{AgentState, MessageStore, StateStore, ThreadId};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SqliteMessageStore {
    db: Arc<Mutex<rusqlite::Connection>>,
}

impl SqliteMessageStore {
    pub fn new(db: Arc<Mutex<rusqlite::Connection>>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl MessageStore for SqliteMessageStore {
    async fn append(&self, thread_id: &ThreadId, message: Message) -> anyhow::Result<()> {
        let db = self.db.lock().await;
        let id = uuid::Uuid::new_v4().to_string();
        let content = serde_json::to_string(&message)?;
        db.execute(
            "INSERT INTO agent_messages (id, thread_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, thread_id.to_string(), content],
        )?;
        Ok(())
    }

    async fn get_history(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<Message>> {
        let db = self.db.lock().await;
        let mut stmt = db.prepare(
            "SELECT data FROM agent_messages WHERE thread_id = ?1 ORDER BY rowid ASC",
        )?;
        let messages = stmt
            .query_map(rusqlite::params![thread_id.to_string()], |row| {
                let data: String = row.get(0)?;
                Ok(data)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|data| serde_json::from_str(&data).ok())
            .collect();
        Ok(messages)
    }

    async fn clear(&self, thread_id: &ThreadId) -> anyhow::Result<()> {
        let db = self.db.lock().await;
        db.execute(
            "DELETE FROM agent_messages WHERE thread_id = ?1",
            rusqlite::params![thread_id.to_string()],
        )?;
        Ok(())
    }

    async fn replace_history(
        &self,
        thread_id: &ThreadId,
        messages: Vec<Message>,
    ) -> anyhow::Result<()> {
        self.clear(thread_id).await?;
        for msg in messages {
            self.append(thread_id, msg).await?;
        }
        Ok(())
    }
}

pub struct SqliteStateStore {
    db: Arc<Mutex<rusqlite::Connection>>,
}

impl SqliteStateStore {
    pub fn new(db: Arc<Mutex<rusqlite::Connection>>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl StateStore for SqliteStateStore {
    async fn save(&self, state: &AgentState) -> anyhow::Result<()> {
        let db = self.db.lock().await;
        let data = serde_json::to_string(state)?;
        db.execute(
            "INSERT OR REPLACE INTO agent_states (thread_id, data, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                state.thread_id.to_string(),
                data,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    async fn load(&self, thread_id: &ThreadId) -> anyhow::Result<Option<AgentState>> {
        let db = self.db.lock().await;
        let mut stmt =
            db.prepare("SELECT data FROM agent_states WHERE thread_id = ?1")?;
        let result = stmt.query_row(rusqlite::params![thread_id.to_string()], |row| {
            let data: String = row.get(0)?;
            Ok(data)
        });
        match result {
            Ok(data) => Ok(serde_json::from_str(&data)?),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn delete(&self, thread_id: &ThreadId) -> anyhow::Result<()> {
        let db = self.db.lock().await;
        db.execute(
            "DELETE FROM agent_states WHERE thread_id = ?1",
            rusqlite::params![thread_id.to_string()],
        )?;
        Ok(())
    }
}
