//! Tests for SoulManager — soul file I/O operations.

use std::sync::Arc;
use tempfile::TempDir;

use claw_agent_rs::soul::SoulManager;

fn setup() -> (TempDir, Arc<SoulManager>) {
    let tmp = TempDir::new().unwrap();
    let soul_dir = tmp.path().join("soul");
    std::fs::create_dir_all(soul_dir.join("memory")).unwrap();
    std::fs::write(soul_dir.join("SOUL.md"), "# Soul\n\n## Personality\n- friendly\n").unwrap();
    std::fs::write(soul_dir.join("BOOTSTRAP.md"), "# Bootstrap\nfirst run\n").unwrap();
    let manager = Arc::new(SoulManager::new(&soul_dir));
    (tmp, manager)
}

// ─── read ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn read_existing_file() {
    let (_tmp, soul) = setup();
    let content = soul.read("SOUL.md").await.unwrap();
    assert!(content.contains("# Soul"));
    assert!(content.contains("friendly"));
}

#[tokio::test]
async fn read_missing_file_returns_error() {
    let (_tmp, soul) = setup();
    let result = soul.read("NONEXISTENT.md").await;
    assert!(result.is_err());
}

// ─── write ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn write_creates_new_file() {
    let (_tmp, soul) = setup();
    soul.write("NEW.md", "# New File\nhello\n").await.unwrap();
    let content = soul.read("NEW.md").await.unwrap();
    assert!(content.contains("# New File"));
}

#[tokio::test]
async fn write_overwrites_existing_file() {
    let (_tmp, soul) = setup();
    soul.write("SOUL.md", "completely new content").await.unwrap();
    let content = soul.read("SOUL.md").await.unwrap();
    assert_eq!(content, "completely new content");
    assert!(!content.contains("friendly"));
}

#[tokio::test]
async fn write_creates_parent_directories() {
    let (_tmp, soul) = setup();
    soul.write("deep/nested/file.md", "nested content").await.unwrap();
    let content = soul.read("deep/nested/file.md").await.unwrap();
    assert_eq!(content, "nested content");
}

// ─── update_section ──────────────────────────────────────────────────────

#[tokio::test]
async fn update_section_replaces_existing() {
    let (_tmp, soul) = setup();
    soul.update_section("SOUL.md", "Personality", "- curious\n- bold").await.unwrap();
    let content = soul.read("SOUL.md").await.unwrap();
    assert!(content.contains("curious"));
    assert!(content.contains("bold"));
    assert!(!content.contains("friendly"));
}

#[tokio::test]
async fn update_section_appends_new_section() {
    let (_tmp, soul) = setup();
    soul.update_section("SOUL.md", "Values", "- honesty\n- simplicity").await.unwrap();
    let content = soul.read("SOUL.md").await.unwrap();
    assert!(content.contains("## Values"));
    assert!(content.contains("honesty"));
    // Original content preserved
    assert!(content.contains("## Personality"));
}

#[tokio::test]
async fn update_section_creates_file_if_missing() {
    let (_tmp, soul) = setup();
    soul.update_section("BRAND_NEW.md", "Section", "content here").await.unwrap();
    let content = soul.read("BRAND_NEW.md").await.unwrap();
    assert!(content.contains("## Section"));
    assert!(content.contains("content here"));
}

// ─── delete ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn delete_bootstrap_succeeds() {
    let (_tmp, soul) = setup();
    assert!(soul.exists("BOOTSTRAP.md"));
    soul.delete("BOOTSTRAP.md").await.unwrap();
    assert!(!soul.exists("BOOTSTRAP.md"));
}

#[tokio::test]
async fn delete_non_bootstrap_fails() {
    let (_tmp, soul) = setup();
    let result = soul.delete("SOUL.md").await;
    assert!(result.is_err());
    assert!(soul.exists("SOUL.md"));
}

#[tokio::test]
async fn delete_missing_bootstrap_is_ok() {
    let (_tmp, soul) = setup();
    soul.delete("BOOTSTRAP.md").await.unwrap();
    // Second delete should also succeed (file already gone)
    soul.delete("BOOTSTRAP.md").await.unwrap();
}

// ─── exists ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn exists_returns_true_for_existing() {
    let (_tmp, soul) = setup();
    assert!(soul.exists("SOUL.md"));
}

#[tokio::test]
async fn exists_returns_false_for_missing() {
    let (_tmp, soul) = setup();
    assert!(!soul.exists("NOPE.md"));
}

// ─── list_daily_logs ─────────────────────────────────────────────────────

#[tokio::test]
async fn list_daily_logs_empty() {
    let (_tmp, soul) = setup();
    let logs = soul.list_daily_logs(7).await.unwrap();
    assert!(logs.is_empty());
}

#[tokio::test]
async fn list_daily_logs_returns_sorted_desc() {
    let (_tmp, soul) = setup();
    soul.write("memory/2026-03-10.md", "day 10").await.unwrap();
    soul.write("memory/2026-03-12.md", "day 12").await.unwrap();
    soul.write("memory/2026-03-11.md", "day 11").await.unwrap();

    let logs = soul.list_daily_logs(7).await.unwrap();
    assert_eq!(logs.len(), 3);
    assert_eq!(logs[0].0, "2026-03-12.md"); // newest first
    assert_eq!(logs[1].0, "2026-03-11.md");
    assert_eq!(logs[2].0, "2026-03-10.md");
}

#[tokio::test]
async fn list_daily_logs_respects_limit() {
    let (_tmp, soul) = setup();
    soul.write("memory/2026-03-10.md", "d10").await.unwrap();
    soul.write("memory/2026-03-11.md", "d11").await.unwrap();
    soul.write("memory/2026-03-12.md", "d12").await.unwrap();

    let logs = soul.list_daily_logs(2).await.unwrap();
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0].0, "2026-03-12.md");
    assert_eq!(logs[1].0, "2026-03-11.md");
}
