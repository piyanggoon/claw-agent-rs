//! Tests for ClawConfig — environment-based configuration.

use std::path::PathBuf;
use claw_agent_rs::config::ClawConfig;

#[test]
fn from_env_does_not_panic() {
    // from_env() reads env vars — we can only assert it doesn't panic
    // and returns a valid struct. Specific values may vary per environment.
    let config = ClawConfig::from_env();

    // These fields are always set (either from env or hard-coded defaults)
    assert!(config.web_port > 0);
    assert!(!config.default_model.is_empty());
    assert!(!config.timezone.is_empty());
    assert!(!config.main_group.is_empty());
}

#[test]
fn soul_dir_computed_correctly() {
    let config = ClawConfig {
        data_dir: PathBuf::from("./data"),
        groups_dir: PathBuf::from("./groups"),
        main_group: "test-group".to_string(),
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        default_model: "test".to_string(),
        web_port: 3100,
        timezone: "UTC".to_string(),
        auth_enabled: false,
        auth_password: None,
        auth_secret: String::new(),
        scheduler_poll_interval: std::time::Duration::from_secs(15),
        max_concurrent_tasks: 3,
        agent_timeout: std::time::Duration::from_secs(300),
    };

    let soul_dir = config.soul_dir();
    assert_eq!(soul_dir, PathBuf::from("./groups/test-group/soul"));
}

#[test]
fn soul_dir_different_groups() {
    let make_config = |group: &str| ClawConfig {
        data_dir: PathBuf::from("./data"),
        groups_dir: PathBuf::from("/custom/groups"),
        main_group: group.to_string(),
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        default_model: "test".to_string(),
        web_port: 3100,
        timezone: "UTC".to_string(),
        auth_enabled: false,
        auth_password: None,
        auth_secret: String::new(),
        scheduler_poll_interval: std::time::Duration::from_secs(15),
        max_concurrent_tasks: 3,
        agent_timeout: std::time::Duration::from_secs(300),
    };

    assert_eq!(make_config("main").soul_dir(), PathBuf::from("/custom/groups/main/soul"));
    assert_eq!(make_config("work").soul_dir(), PathBuf::from("/custom/groups/work/soul"));
    assert_eq!(make_config("personal").soul_dir(), PathBuf::from("/custom/groups/personal/soul"));
}
