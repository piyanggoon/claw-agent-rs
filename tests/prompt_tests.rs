//! Tests for system prompt builder.

use std::sync::Arc;
use tempfile::TempDir;

use claw_agent_rs::memory::MemoryManager;
use claw_agent_rs::soul::SoulManager;
use claw_agent_rs::soul::prompt::build_system_prompt;

fn setup() -> (TempDir, Arc<SoulManager>, Arc<MemoryManager>) {
    let tmp = TempDir::new().unwrap();
    let soul_dir = tmp.path().join("soul");
    std::fs::create_dir_all(soul_dir.join("memory")).unwrap();

    std::fs::write(soul_dir.join("SOUL.md"), "# Soul\n\n## Personality\n- friendly\n").unwrap();
    std::fs::write(soul_dir.join("IDENTITY.md"), "# Identity\n\n## Name\nClaw\n").unwrap();
    std::fs::write(soul_dir.join("USER.md"), "# User\n\n## Profile\n- Name: Zent\n").unwrap();
    std::fs::write(soul_dir.join("MEMORY.md"), "# Memory\n\n## Facts\n- test fact\n").unwrap();
    std::fs::write(soul_dir.join("HEARTBEAT.md"), "# Heartbeat\n").unwrap();
    std::fs::write(soul_dir.join("TOOLS.md"), "# Tools\n").unwrap();

    let soul = Arc::new(SoulManager::new(&soul_dir));
    let memory = Arc::new(MemoryManager::new(soul.clone()));
    (tmp, soul, memory)
}

#[tokio::test]
async fn prompt_includes_all_soul_files() {
    let (tmp, soul, memory) = setup();
    let agents_md = tmp.path().join("AGENTS.md");
    std::fs::write(&agents_md, "# Agent Instructions\nYou are Claw.\n").unwrap();

    let prompt = build_system_prompt(&soul, &memory, "Asia/Bangkok", &agents_md).await;

    // AGENTS.md
    assert!(prompt.contains("<!-- AGENTS.md -->"));
    assert!(prompt.contains("You are Claw"));
    assert!(prompt.contains("<!-- /AGENTS.md -->"));

    // Soul files
    assert!(prompt.contains("<!-- SOUL.md -->"));
    assert!(prompt.contains("friendly"));
    assert!(prompt.contains("<!-- IDENTITY.md -->"));
    assert!(prompt.contains("Claw"));
    assert!(prompt.contains("<!-- USER.md -->"));
    assert!(prompt.contains("Zent"));
    assert!(prompt.contains("<!-- MEMORY.md -->"));
    assert!(prompt.contains("test fact"));
    assert!(prompt.contains("<!-- HEARTBEAT.md -->"));
    assert!(prompt.contains("<!-- TOOLS.md -->"));
}

#[tokio::test]
async fn prompt_includes_current_datetime() {
    let (tmp, soul, memory) = setup();
    let agents_md = tmp.path().join("AGENTS.md");
    std::fs::write(&agents_md, "instructions").unwrap();

    let prompt = build_system_prompt(&soul, &memory, "UTC", &agents_md).await;
    assert!(prompt.contains("<!-- context -->"));
    assert!(prompt.contains("Current date/time:"));
    assert!(prompt.contains("UTC"));
}

#[tokio::test]
async fn prompt_includes_bootstrap_when_exists() {
    let (tmp, soul, memory) = setup();
    let agents_md = tmp.path().join("AGENTS.md");
    std::fs::write(&agents_md, "instructions").unwrap();

    // Write BOOTSTRAP.md
    soul.write("BOOTSTRAP.md", "# Bootstrap\nFirst run steps.\n").await.unwrap();

    let prompt = build_system_prompt(&soul, &memory, "UTC", &agents_md).await;
    assert!(prompt.contains("<!-- BOOTSTRAP.md -->"));
    assert!(prompt.contains("First run steps"));
}

#[tokio::test]
async fn prompt_excludes_bootstrap_when_missing() {
    let (tmp, soul, memory) = setup();
    let agents_md = tmp.path().join("AGENTS.md");
    std::fs::write(&agents_md, "instructions").unwrap();

    // No BOOTSTRAP.md
    let prompt = build_system_prompt(&soul, &memory, "UTC", &agents_md).await;
    assert!(!prompt.contains("<!-- BOOTSTRAP.md -->"));
}

#[tokio::test]
async fn prompt_includes_daily_logs() {
    let (tmp, soul, memory) = setup();
    let agents_md = tmp.path().join("AGENTS.md");
    std::fs::write(&agents_md, "instructions").unwrap();

    // Write a daily log
    memory.daily_log("user asked about Rust", Some("interaction")).await.unwrap();

    let prompt = build_system_prompt(&soul, &memory, "UTC", &agents_md).await;
    assert!(prompt.contains("Recent Daily Logs"));
    assert!(prompt.contains("user asked about Rust"));
}

#[tokio::test]
async fn prompt_gracefully_handles_missing_agents_md() {
    let (tmp, soul, memory) = setup();
    let agents_md = tmp.path().join("NONEXISTENT_AGENTS.md");

    // Should not panic — just skip the missing file
    let prompt = build_system_prompt(&soul, &memory, "UTC", &agents_md).await;
    assert!(!prompt.contains("AGENTS.md"));
    // But soul files should still be there
    assert!(prompt.contains("<!-- SOUL.md -->"));
}

#[tokio::test]
async fn prompt_order_agents_first_then_soul() {
    let (tmp, soul, memory) = setup();
    let agents_md = tmp.path().join("AGENTS.md");
    std::fs::write(&agents_md, "AGENTS_MARKER_FIRST").unwrap();

    let prompt = build_system_prompt(&soul, &memory, "UTC", &agents_md).await;

    let agents_pos = prompt.find("AGENTS_MARKER_FIRST").unwrap();
    let soul_pos = prompt.find("<!-- SOUL.md -->").unwrap();
    assert!(agents_pos < soul_pos, "AGENTS.md should come before SOUL.md");
}
