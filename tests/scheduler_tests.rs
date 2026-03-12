//! Tests for scheduler utility functions and paginated message queries.
//!
//! Covers `calculate_next_run`, `calculate_initial_next_run` from
//! `scheduler::engine`, and `get_messages_paginated` from `db::messages`.

use claw_agent_rs::db::{messages, schema, sessions};
use claw_agent_rs::scheduler::engine::{calculate_initial_next_run, calculate_next_run};

// ═══════════════════════════════════════════════════════════════════════════
// calculate_next_run
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn calculate_next_run_cron() {
    // "every minute" — should always produce a valid future timestamp
    let result = calculate_next_run("cron", "0 * * * * *");
    assert!(result.is_some(), "cron should produce Some");
    let next = result.unwrap();
    // Should be valid RFC 3339
    let parsed = chrono::DateTime::parse_from_rfc3339(&next);
    assert!(parsed.is_ok(), "cron next_run should be valid RFC 3339: {next}");
    // Should be in the future
    assert!(parsed.unwrap() > chrono::Utc::now());
}

#[test]
fn calculate_next_run_cron_hourly() {
    let result = calculate_next_run("cron", "0 0 * * * *");
    assert!(result.is_some());
    let parsed = chrono::DateTime::parse_from_rfc3339(&result.unwrap());
    assert!(parsed.is_ok());
}

#[test]
fn calculate_next_run_cron_invalid_returns_none() {
    let result = calculate_next_run("cron", "not a cron expression");
    assert!(result.is_none(), "invalid cron should return None");
}

#[test]
fn calculate_next_run_interval() {
    let before = chrono::Utc::now();
    let result = calculate_next_run("interval", "60000");
    assert!(result.is_some(), "interval should produce Some");

    let next = chrono::DateTime::parse_from_rfc3339(&result.unwrap()).unwrap();
    let after = chrono::Utc::now();

    // next should be approximately now + 60 seconds
    let expected_min = before + chrono::Duration::milliseconds(60000);
    let expected_max = after + chrono::Duration::milliseconds(60000);
    assert!(next >= expected_min, "interval next_run too early");
    assert!(next <= expected_max, "interval next_run too late");
}

#[test]
fn calculate_next_run_interval_small() {
    let before = chrono::Utc::now();
    let result = calculate_next_run("interval", "1000");
    assert!(result.is_some());
    let next = chrono::DateTime::parse_from_rfc3339(&result.unwrap()).unwrap();
    // Should be ~1 second from now
    let diff = next.signed_duration_since(before);
    assert!(diff.num_milliseconds() >= 900 && diff.num_milliseconds() <= 2000);
}

#[test]
fn calculate_next_run_interval_invalid_returns_none() {
    let result = calculate_next_run("interval", "not_a_number");
    assert!(result.is_none(), "invalid interval should return None");
}

#[test]
fn calculate_next_run_once_returns_none() {
    let result = calculate_next_run("once", "2099-01-01T00:00:00+00:00");
    assert!(result.is_none(), "once should return None (one-shot)");
}

#[test]
fn calculate_next_run_delay_returns_none() {
    let result = calculate_next_run("delay", "5000");
    assert!(result.is_none(), "delay should return None (one-shot)");
}

#[test]
fn calculate_next_run_unknown_type_returns_none() {
    let result = calculate_next_run("weekly", "7");
    assert!(result.is_none(), "unknown schedule type should return None");
}

// ═══════════════════════════════════════════════════════════════════════════
// calculate_initial_next_run
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn calculate_initial_next_run_delay() {
    let before = chrono::Utc::now();
    let result = calculate_initial_next_run("delay", "30000");
    assert!(result.is_some(), "delay should produce Some for initial");

    let next = chrono::DateTime::parse_from_rfc3339(&result.unwrap()).unwrap();
    let after = chrono::Utc::now();

    // Should be approximately now + 30 seconds
    let expected_min = before + chrono::Duration::milliseconds(30000);
    let expected_max = after + chrono::Duration::milliseconds(30000);
    assert!(next >= expected_min);
    assert!(next <= expected_max);
}

#[test]
fn calculate_initial_next_run_delay_invalid_returns_none() {
    let result = calculate_initial_next_run("delay", "abc");
    assert!(result.is_none());
}

#[test]
fn calculate_initial_next_run_once() {
    let ts = "2099-06-15T12:00:00+00:00";
    let result = calculate_initial_next_run("once", ts);
    assert!(result.is_some(), "once should return the value as-is");
    assert_eq!(result.unwrap(), ts);
}

#[test]
fn calculate_initial_next_run_once_arbitrary_string() {
    // "once" just passes through the value verbatim
    let val = "2026-03-12T10:30:00";
    let result = calculate_initial_next_run("once", val);
    assert_eq!(result, Some(val.to_string()));
}

#[test]
fn calculate_initial_next_run_cron() {
    let result = calculate_initial_next_run("cron", "0 * * * * *");
    assert!(result.is_some(), "cron should produce a next occurrence");
    let next = chrono::DateTime::parse_from_rfc3339(&result.unwrap()).unwrap();
    assert!(next > chrono::Utc::now(), "cron initial should be in the future");
}

#[test]
fn calculate_initial_next_run_cron_invalid_returns_none() {
    let result = calculate_initial_next_run("cron", "garbage");
    assert!(result.is_none());
}

#[test]
fn calculate_initial_next_run_interval() {
    let before = chrono::Utc::now();
    let result = calculate_initial_next_run("interval", "120000");
    assert!(result.is_some(), "interval should produce Some for initial");

    let next = chrono::DateTime::parse_from_rfc3339(&result.unwrap()).unwrap();
    let after = chrono::Utc::now();

    let expected_min = before + chrono::Duration::milliseconds(120000);
    let expected_max = after + chrono::Duration::milliseconds(120000);
    assert!(next >= expected_min);
    assert!(next <= expected_max);
}

#[test]
fn calculate_initial_next_run_interval_invalid_returns_none() {
    let result = calculate_initial_next_run("interval", "not_num");
    assert!(result.is_none());
}

#[test]
fn calculate_initial_next_run_unknown_type_returns_none() {
    let result = calculate_initial_next_run("biweekly", "14");
    assert!(result.is_none(), "unknown type should return None");
}

// ═══════════════════════════════════════════════════════════════════════════
// get_messages_paginated (DB integration tests)
// ═══════════════════════════════════════════════════════════════════════════

fn setup_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .unwrap();
    schema::initialize_db(&conn).unwrap();
    conn
}

#[test]
fn paginated_without_cursor_returns_most_recent() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Test")).unwrap();

    for i in 0..5 {
        messages::store_message(
            &conn,
            &format!("m{i}"),
            "t1",
            "user",
            &format!("message {i}"),
            Some("s1"),
            None,
        )
        .unwrap();
        // Small delay to ensure distinct timestamps
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let page = messages::get_messages_paginated(&conn, "s1", 3, None).unwrap();
    assert_eq!(page.len(), 3);
    // Without cursor, returns the 3 MOST RECENT messages in chronological order
    assert_eq!(page[0].content, "message 2");
    assert_eq!(page[1].content, "message 3");
    assert_eq!(page[2].content, "message 4");
}

#[test]
fn paginated_with_before_cursor() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Test")).unwrap();

    for i in 0..5 {
        messages::store_message(
            &conn,
            &format!("m{i}"),
            "t1",
            "user",
            &format!("message {i}"),
            Some("s1"),
            None,
        )
        .unwrap();
        // Ensure distinct timestamps so the cursor query works correctly
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Get timestamp of m3 to use as cursor (frontend sends timestamps, not IDs)
    let all = messages::get_messages_by_session(&conn, "s1", Some(100), None).unwrap();
    let m3_ts = &all[3].timestamp;

    // Get messages before m3's timestamp — should return m0, m1, m2 in chronological order
    let page = messages::get_messages_paginated(&conn, "s1", 10, Some(m3_ts)).unwrap();
    assert_eq!(page.len(), 3);
    // Results are in chronological order (reversed from DESC query)
    assert_eq!(page[0].id, "m0");
    assert_eq!(page[1].id, "m1");
    assert_eq!(page[2].id, "m2");
}

#[test]
fn paginated_returns_correct_limit() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Test")).unwrap();

    for i in 0..10 {
        messages::store_message(
            &conn,
            &format!("m{i}"),
            "t1",
            "user",
            &format!("msg {i}"),
            Some("s1"),
            None,
        )
        .unwrap();
    }

    let page = messages::get_messages_paginated(&conn, "s1", 4, None).unwrap();
    assert_eq!(page.len(), 4, "should return exactly the requested limit");
}

#[test]
fn paginated_limit_greater_than_total() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Test")).unwrap();

    messages::store_message(&conn, "m0", "t1", "user", "only one", Some("s1"), None).unwrap();

    let page = messages::get_messages_paginated(&conn, "s1", 100, None).unwrap();
    assert_eq!(page.len(), 1, "should return all available when limit > total");
}

#[test]
fn paginated_empty_session() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Empty")).unwrap();

    let page = messages::get_messages_paginated(&conn, "s1", 10, None).unwrap();
    assert!(page.is_empty(), "empty session should return empty vec");
}

#[test]
fn paginated_nonexistent_session() {
    let conn = setup_db();

    let page = messages::get_messages_paginated(&conn, "no_such_session", 10, None).unwrap();
    assert!(page.is_empty());
}

#[test]
fn paginated_before_first_message_returns_empty() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Test")).unwrap();

    messages::store_message(&conn, "m0", "t1", "user", "first", Some("s1"), None).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    messages::store_message(&conn, "m1", "t1", "user", "second", Some("s1"), None).unwrap();

    // Use m0's timestamp as cursor — nothing should be before the first message
    let all = messages::get_messages_by_session(&conn, "s1", Some(100), None).unwrap();
    let m0_ts = &all[0].timestamp;
    let page = messages::get_messages_paginated(&conn, "s1", 10, Some(m0_ts)).unwrap();
    assert!(page.is_empty(), "nothing should be before the first message");
}

#[test]
fn has_messages_before_works() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Test")).unwrap();

    messages::store_message(&conn, "m0", "t1", "user", "first", Some("s1"), None).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    messages::store_message(&conn, "m1", "t1", "user", "second", Some("s1"), None).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    messages::store_message(&conn, "m2", "t1", "user", "third", Some("s1"), None).unwrap();

    let all = messages::get_messages_by_session(&conn, "s1", Some(100), None).unwrap();

    // Before m0 → nothing
    assert!(!messages::has_messages_before(&conn, "s1", &all[0].timestamp).unwrap());
    // Before m1 → m0 exists
    assert!(messages::has_messages_before(&conn, "s1", &all[1].timestamp).unwrap());
    // Before m2 → m0, m1 exist
    assert!(messages::has_messages_before(&conn, "s1", &all[2].timestamp).unwrap());
}

/// Regression: new-format timestamps (%.3fZ) must not produce duplicates.
/// Frontend sends `new Date(ts).toISOString()` which is identical to the
/// stored format, so `<` comparison excludes the cursor message correctly.
#[test]
fn paginated_cursor_no_duplicate_with_js_format() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Test")).unwrap();

    for i in 0..5 {
        messages::store_message(
            &conn,
            &format!("m{i}"),
            "t1",
            "user",
            &format!("msg {i}"),
            Some("s1"),
            None,
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Load initial page (most recent 3)
    let page1 = messages::get_messages_paginated(&conn, "s1", 3, None).unwrap();
    assert_eq!(page1.len(), 3);
    let oldest = &page1[0]; // This is the cursor boundary

    // Simulate frontend: new Date(ts).toISOString()
    let js_cursor = {
        let dt = chrono::DateTime::parse_from_rfc3339(&oldest.timestamp).unwrap();
        dt.with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string()
    };

    // Load older messages
    let page2 = messages::get_messages_paginated(&conn, "s1", 10, Some(&js_cursor)).unwrap();
    for m in &page2 {
        assert_ne!(m.id, oldest.id, "cursor message must not appear in next page");
    }
    assert_eq!(page2.len(), 2, "should return the 2 messages older than cursor");

    // has_messages_before must also NOT count the cursor message itself
    assert!(!messages::has_messages_before(&conn, "s1", &page2[0].timestamp).unwrap(),
        "nothing should be before the very first message");
}

/// Regression: has_messages_before must stop returning true once all older
/// messages have been loaded (no infinite loop).
#[test]
fn has_messages_before_does_not_cause_infinite_loop() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Test")).unwrap();

    for i in 0..3 {
        messages::store_message(
            &conn,
            &format!("m{i}"),
            "t1",
            "user",
            &format!("msg {i}"),
            Some("s1"),
            None,
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Simulate full pagination flow:
    // Page 1: get most recent 2
    let page1 = messages::get_messages_paginated(&conn, "s1", 2, None).unwrap();
    assert_eq!(page1.len(), 2); // m1, m2
    let has_more_1 = messages::has_messages_before(&conn, "s1", &page1[0].timestamp).unwrap();
    assert!(has_more_1, "should have more messages before page 1");

    // Page 2: use oldest from page1 as cursor (as frontend would via JS Date)
    let cursor = {
        let dt = chrono::DateTime::parse_from_rfc3339(&page1[0].timestamp).unwrap();
        dt.with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string()
    };
    let page2 = messages::get_messages_paginated(&conn, "s1", 10, Some(&cursor)).unwrap();
    assert_eq!(page2.len(), 1); // m0
    let has_more_2 = messages::has_messages_before(&conn, "s1", &page2[0].timestamp).unwrap();
    assert!(!has_more_2, "no messages before the first message — pagination must stop");
}

/// migrate_timestamps converts old +00:00 format to JS Z format.
#[test]
fn migrate_timestamps_converts_old_format() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Test")).unwrap();

    // Manually insert messages with OLD format timestamps (simulating pre-fix data)
    let old_ts_1 = "2026-01-01T10:00:00.123456789+00:00";
    let old_ts_2 = "2026-01-01T10:00:01.987654321+00:00";
    let new_ts = "2026-01-01T10:00:02.500Z"; // already new format
    conn.execute(
        "INSERT INTO messages (id, thread_id, role, content, timestamp, web_session_id) VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params!["m0", "t1", "user", "old 1", old_ts_1, "s1"],
    ).unwrap();
    conn.execute(
        "INSERT INTO messages (id, thread_id, role, content, timestamp, web_session_id) VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params!["m1", "t1", "user", "old 2", old_ts_2, "s1"],
    ).unwrap();
    conn.execute(
        "INSERT INTO messages (id, thread_id, role, content, timestamp, web_session_id) VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params!["m2", "t1", "user", "new", new_ts, "s1"],
    ).unwrap();

    // Run migration
    let migrated = messages::migrate_timestamps(&conn).unwrap();
    assert_eq!(migrated, 2, "should migrate the 2 old-format timestamps");

    // Verify timestamps are now in JS format
    let all = messages::get_messages_by_session(&conn, "s1", Some(100), None).unwrap();
    assert_eq!(all[0].timestamp, "2026-01-01T10:00:00.123Z");
    assert_eq!(all[1].timestamp, "2026-01-01T10:00:01.987Z"); // truncated from .987654
    assert_eq!(all[2].timestamp, "2026-01-01T10:00:02.500Z"); // unchanged

    // Idempotent — running again migrates 0
    let migrated2 = messages::migrate_timestamps(&conn).unwrap();
    assert_eq!(migrated2, 0, "should be idempotent");

    // Pagination with JS cursor should now work correctly
    let cursor = "2026-01-01T10:00:01.988Z"; // m1's new timestamp
    let page = messages::get_messages_paginated(&conn, "s1", 10, Some(cursor)).unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id, "m0", "should return only m0 before the cursor");
}

#[test]
fn session_stats_aggregation() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Stats")).unwrap();

    // User message (no metadata)
    messages::store_message(&conn, "m0", "t1", "user", "hi", Some("s1"), None).unwrap();

    // Assistant message with metadata
    let meta1 = r#"{"toolCalls":[],"resultMeta":{"costUsd":0.01,"inputTokens":500,"outputTokens":200,"numTurns":1}}"#;
    messages::store_message(&conn, "m1", "t1", "assistant", "hello", Some("s1"), Some(meta1)).unwrap();

    // Another assistant message
    let meta2 = r#"{"toolCalls":[{"id":"t1","name":"bash"}],"resultMeta":{"costUsd":0.05,"inputTokens":1000,"outputTokens":800,"numTurns":3}}"#;
    messages::store_message(&conn, "m2", "t1", "assistant", "done", Some("s1"), Some(meta2)).unwrap();

    let (cost, input, output, turns) = messages::get_session_stats(&conn, "s1").unwrap();
    assert!((cost - 0.06).abs() < 1e-9, "total cost: {cost}");
    assert_eq!(input, 1500);
    assert_eq!(output, 1000);
    assert_eq!(turns, 4);
}
