//! Tests for MemoryManager — save, recall, forget, daily_log.

use std::sync::Arc;
use tempfile::TempDir;

use claw_agent_rs::memory::MemoryManager;
use claw_agent_rs::soul::SoulManager;

fn setup() -> (TempDir, Arc<MemoryManager>, Arc<SoulManager>) {
    let tmp = TempDir::new().unwrap();
    let soul_dir = tmp.path().join("soul");
    std::fs::create_dir_all(soul_dir.join("memory")).unwrap();
    std::fs::write(
        soul_dir.join("MEMORY.md"),
        "# Memory\n\n## Facts\n- user name is Zent\n- timezone is Bangkok\n\n## Preferences\n- dark mode\n",
    ).unwrap();
    let soul = Arc::new(SoulManager::new(&soul_dir));
    let memory = Arc::new(MemoryManager::new(soul.clone()));
    (tmp, memory, soul)
}

// ─── save (append) ───────────────────────────────────────────────────────

#[tokio::test]
async fn save_append_adds_to_section() {
    let (_tmp, memory, soul) = setup();
    memory.save("Facts", "- likes Rust", "append").await.unwrap();
    let content = soul.read("MEMORY.md").await.unwrap();
    assert!(content.contains("user name is Zent")); // old content preserved
    assert!(content.contains("likes Rust")); // new content appended
}

#[tokio::test]
async fn save_append_creates_section_if_missing() {
    let (_tmp, memory, soul) = setup();
    memory.save("Insights", "- first insight", "append").await.unwrap();
    let content = soul.read("MEMORY.md").await.unwrap();
    assert!(content.contains("## Insights"));
    assert!(content.contains("first insight"));
}

// ─── save (replace) ──────────────────────────────────────────────────────

#[tokio::test]
async fn save_replace_overwrites_section() {
    let (_tmp, memory, soul) = setup();
    memory.save("Facts", "- only this remains", "replace").await.unwrap();
    let content = soul.read("MEMORY.md").await.unwrap();
    assert!(content.contains("only this remains"));
    assert!(!content.contains("user name is Zent")); // old facts gone
    assert!(content.contains("dark mode")); // other sections preserved
}

// ─── daily_log ───────────────────────────────────────────────────────────

#[tokio::test]
async fn daily_log_creates_file_with_header() {
    let (_tmp, memory, _) = setup();
    memory.daily_log("first entry", None).await.unwrap();
    let logs = memory.get_recent_daily_logs(1).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert!(logs[0].1.contains("# Daily Log"));
    assert!(logs[0].1.contains("first entry"));
}

#[tokio::test]
async fn daily_log_with_category() {
    let (_tmp, memory, _) = setup();
    memory.daily_log("something happened", Some("event")).await.unwrap();
    let logs = memory.get_recent_daily_logs(1).await.unwrap();
    assert!(logs[0].1.contains("[event]"));
    assert!(logs[0].1.contains("something happened"));
}

#[tokio::test]
async fn daily_log_appends_multiple_entries() {
    let (_tmp, memory, _) = setup();
    memory.daily_log("entry 1", None).await.unwrap();
    memory.daily_log("entry 2", Some("observation")).await.unwrap();
    memory.daily_log("entry 3", Some("decision")).await.unwrap();

    let logs = memory.get_recent_daily_logs(1).await.unwrap();
    let content = &logs[0].1;
    assert!(content.contains("entry 1"));
    assert!(content.contains("entry 2"));
    assert!(content.contains("entry 3"));
    assert!(content.contains("[observation]"));
    assert!(content.contains("[decision]"));
}

// ─── recall ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn recall_without_query_returns_all() {
    let (_tmp, memory, _) = setup();
    let result = memory.recall(None, 7).await.unwrap();
    assert!(result.contains("user name is Zent"));
    assert!(result.contains("dark mode"));
}

#[tokio::test]
async fn recall_with_query_filters() {
    let (_tmp, memory, _) = setup();
    let result = memory.recall(Some("timezone"), 7).await.unwrap();
    assert!(result.contains("Bangkok"));
    assert!(!result.contains("dark mode")); // unmatched lines excluded
}

#[tokio::test]
async fn recall_no_matches_returns_message() {
    let (_tmp, memory, _) = setup();
    let result = memory.recall(Some("xyznonexistent"), 7).await.unwrap();
    assert!(result.to_lowercase().contains("no matching"));
}

#[tokio::test]
async fn recall_includes_daily_logs() {
    let (_tmp, memory, _) = setup();
    memory.daily_log("special event happened", Some("event")).await.unwrap();
    let result = memory.recall(Some("special event"), 7).await.unwrap();
    assert!(result.contains("special event"));
}

// ─── forget ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn forget_removes_matching_entry() {
    let (_tmp, memory, soul) = setup();
    memory.forget("Facts", "timezone").await.unwrap();
    let content = soul.read("MEMORY.md").await.unwrap();
    assert!(!content.contains("timezone is Bangkok"));
    assert!(content.contains("user name is Zent")); // other entries preserved
}

#[tokio::test]
async fn forget_case_insensitive() {
    let (_tmp, memory, soul) = setup();
    memory.forget("Facts", "ZENT").await.unwrap();
    let content = soul.read("MEMORY.md").await.unwrap();
    assert!(!content.contains("user name is Zent"));
}

#[tokio::test]
async fn forget_no_match_is_ok() {
    let (_tmp, memory, soul) = setup();
    let before = soul.read("MEMORY.md").await.unwrap();
    memory.forget("Facts", "nonexistent entry xyz").await.unwrap();
    let after = soul.read("MEMORY.md").await.unwrap();
    // Content should be essentially the same
    assert_eq!(before.trim(), after.trim());
}

#[tokio::test]
async fn forget_only_affects_target_section() {
    let (_tmp, memory, soul) = setup();
    // "dark mode" is in Preferences, not Facts — should NOT be removed
    memory.forget("Facts", "dark mode").await.unwrap();
    let content = soul.read("MEMORY.md").await.unwrap();
    assert!(content.contains("dark mode")); // still in Preferences
}
