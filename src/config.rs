use std::env;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct ClawConfig {
    pub data_dir: PathBuf,
    pub groups_dir: PathBuf,
    pub main_group: String,

    // LLM keys
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub google_api_key: Option<String>,

    pub default_model: String,

    // Server
    pub web_port: u16,
    pub timezone: String,

    // Authentication
    /// Whether authentication is required for the web UI.
    pub auth_enabled: bool,
    /// The password users must enter to access the web UI.
    pub auth_password: Option<String>,
    /// Secret key for HMAC-SHA256 token signing.
    /// If not set explicitly, derived from `auth_password` as `claw-auth-{password}-secret-key`.
    pub auth_secret: String,

    // Scheduler
    pub scheduler_poll_interval: Duration,
    pub max_concurrent_tasks: usize,
    pub agent_timeout: Duration,
}

impl ClawConfig {
    /// Load configuration from environment variables with sensible defaults.
    ///
    /// | Variable                  | Default            |
    /// |---------------------------|--------------------|
    /// | `DATA_DIR`                | `"./data"`         |
    /// | `GROUPS_DIR`              | `"./groups"`       |
    /// | `MAIN_GROUP`              | `"main"`           |
    /// | `ANTHROPIC_API_KEY`       | —                  |
    /// | `OPENAI_API_KEY`          | —                  |
    /// | `GOOGLE_API_KEY`          | —                  |
    /// | `DEFAULT_MODEL`           | `"claude-sonnet-4-6"` |
    /// | `WEB_PORT`                | `3100`             |
    /// | `TIMEZONE`                | `"Asia/Bangkok"`   |
    /// | `AUTH_ENABLED`            | `"0"`              |
    /// | `AUTH_PASSWORD`           | —                  |
    /// | `AUTH_SECRET`             | derived from password |
    /// | `SCHEDULER_POLL_INTERVAL` | `15` (seconds)     |
    /// | `MAX_CONCURRENT_TASKS`    | `3`                |
    /// | `AGENT_TIMEOUT`           | `300` (seconds)    |
    pub fn from_env() -> Self {
        let auth_password = env::var("AUTH_PASSWORD").ok();
        let auth_secret = env::var("AUTH_SECRET").unwrap_or_else(|_| {
            // Derive from password, matching SoulClaw frontend convention
            match &auth_password {
                Some(pw) => format!("claw-auth-{pw}-secret-key"),
                None => String::new(),
            }
        });
        let auth_enabled = env::var("AUTH_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Self {
            data_dir: PathBuf::from(
                env::var("DATA_DIR").unwrap_or_else(|_| "./data".into()),
            ),

            groups_dir: PathBuf::from(
                env::var("GROUPS_DIR").unwrap_or_else(|_| "./groups".into()),
            ),

            main_group: env::var("MAIN_GROUP")
                .unwrap_or_else(|_| "main".into()),

            anthropic_api_key: env::var("ANTHROPIC_API_KEY").ok(),
            openai_api_key: env::var("OPENAI_API_KEY").ok(),
            google_api_key: env::var("GOOGLE_API_KEY").ok(),

            default_model: env::var("DEFAULT_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-6".into()),

            web_port: env::var("WEB_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3100),

            timezone: env::var("TIMEZONE")
                .unwrap_or_else(|_| "Asia/Bangkok".into()),

            auth_enabled,
            auth_password,
            auth_secret,

            scheduler_poll_interval: Duration::from_secs(
                env::var("SCHEDULER_POLL_INTERVAL")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(15),
            ),

            max_concurrent_tasks: env::var("MAX_CONCURRENT_TASKS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),

            agent_timeout: Duration::from_secs(
                env::var("AGENT_TIMEOUT")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(300),
            ),
        }
    }

    /// Returns the path to the soul directory for the main group:
    /// `<groups_dir>/<main_group>/soul`
    pub fn soul_dir(&self) -> PathBuf {
        self.groups_dir.join(&self.main_group).join("soul")
    }
}
