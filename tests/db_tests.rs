//! Tests for database CRUD operations — sessions, messages, tasks, notifications.

use claw_agent_rs::db::{schema, sessions, messages, tasks, notifications};

fn setup_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;").unwrap();
    schema::initialize_db(&conn).unwrap();
    conn
}

// ═══════════════════════════════════════════════════════════════════════════
// SESSIONS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn session_create_and_get() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Test Session")).unwrap();
    let session = sessions::get_session(&conn, "s1").unwrap().unwrap();
    assert_eq!(session.id, "s1");
    assert_eq!(session.title.as_deref(), Some("Test Session"));
}

#[test]
fn session_get_missing_returns_none() {
    let conn = setup_db();
    let session = sessions::get_session(&conn, "nonexistent").unwrap();
    assert!(session.is_none());
}

#[test]
fn session_list_ordered_by_last_message() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("First")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    sessions::create_session(&conn, "s2", Some("Second")).unwrap();

    let list = sessions::get_sessions(&conn).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, "s2"); // most recent first
}

#[test]
fn session_update_title() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", Some("Old")).unwrap();
    sessions::update_session_title(&conn, "s1", "New Title").unwrap();
    let session = sessions::get_session(&conn, "s1").unwrap().unwrap();
    assert_eq!(session.title.as_deref(), Some("New Title"));
}

#[test]
fn session_delete() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", None).unwrap();
    sessions::delete_session(&conn, "s1").unwrap();
    assert!(sessions::get_session(&conn, "s1").unwrap().is_none());
}

#[test]
fn session_touch_updates_timestamp() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", None).unwrap();
    let before = sessions::get_session(&conn, "s1").unwrap().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    sessions::touch_session(&conn, "s1").unwrap();
    let after = sessions::get_session(&conn, "s1").unwrap().unwrap();
    assert_ne!(before.last_message_at, after.last_message_at);
}

// ═══════════════════════════════════════════════════════════════════════════
// MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn message_store_and_get_by_session() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", None).unwrap();
    messages::store_message(&conn, "m1", "t1", "user", "Hello", Some("s1"), None).unwrap();
    messages::store_message(&conn, "m2", "t1", "assistant", "Hi there!", Some("s1"), None).unwrap();

    let msgs = messages::get_messages_by_session(&conn, "s1", None, None).unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[1].role, "assistant");
}

#[test]
fn message_get_by_thread() {
    let conn = setup_db();
    messages::store_message(&conn, "m1", "thread-a", "user", "Hello", None, None).unwrap();
    messages::store_message(&conn, "m2", "thread-b", "user", "Other", None, None).unwrap();

    let msgs = messages::get_messages_by_thread(&conn, "thread-a").unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "Hello");
}

#[test]
fn message_count() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", None).unwrap();
    messages::store_message(&conn, "m1", "t1", "user", "a", Some("s1"), None).unwrap();
    messages::store_message(&conn, "m2", "t1", "user", "b", Some("s1"), None).unwrap();
    messages::store_message(&conn, "m3", "t1", "user", "c", Some("s1"), None).unwrap();

    assert_eq!(messages::count_messages(&conn, "s1").unwrap(), 3);
}

#[test]
fn message_delete_by_session() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", None).unwrap();
    messages::store_message(&conn, "m1", "t1", "user", "a", Some("s1"), None).unwrap();
    messages::store_message(&conn, "m2", "t1", "user", "b", Some("s1"), None).unwrap();

    messages::delete_messages_by_session(&conn, "s1").unwrap();
    assert_eq!(messages::count_messages(&conn, "s1").unwrap(), 0);
}

#[test]
fn message_pagination() {
    let conn = setup_db();
    sessions::create_session(&conn, "s1", None).unwrap();
    for i in 0..10 {
        messages::store_message(&conn, &format!("m{}", i), "t1", "user", &format!("msg {}", i), Some("s1"), None).unwrap();
    }

    let page1 = messages::get_messages_by_session(&conn, "s1", Some(3), Some(0)).unwrap();
    assert_eq!(page1.len(), 3);

    let page2 = messages::get_messages_by_session(&conn, "s1", Some(3), Some(3)).unwrap();
    assert_eq!(page2.len(), 3);
    assert_ne!(page1[0].id, page2[0].id);
}

// ═══════════════════════════════════════════════════════════════════════════
// TASKS
// ═══════════════════════════════════════════════════════════════════════════

fn make_task(id: &str, status: &str) -> tasks::ScheduledTask {
    tasks::ScheduledTask {
        id: id.to_string(),
        group_folder: "main".to_string(),
        prompt: "test prompt".to_string(),
        schedule_type: "delay".to_string(),
        schedule_value: "60000".to_string(),
        context_mode: "isolated".to_string(),
        context_session: None,
        next_run: Some(chrono::Utc::now().to_rfc3339()),
        last_run: None,
        last_result: None,
        status: status.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[test]
fn task_create_and_get() {
    let conn = setup_db();
    let task = make_task("t1", "active");
    tasks::create_task(&conn, &task).unwrap();

    let fetched = tasks::get_task(&conn, "t1").unwrap().unwrap();
    assert_eq!(fetched.prompt, "test prompt");
    assert_eq!(fetched.status, "active");
}

#[test]
fn task_get_missing_returns_none() {
    let conn = setup_db();
    assert!(tasks::get_task(&conn, "nope").unwrap().is_none());
}

#[test]
fn task_list_all() {
    let conn = setup_db();
    tasks::create_task(&conn, &make_task("t1", "active")).unwrap();
    tasks::create_task(&conn, &make_task("t2", "paused")).unwrap();
    let all = tasks::get_all_tasks(&conn).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn task_update_status() {
    let conn = setup_db();
    tasks::create_task(&conn, &make_task("t1", "active")).unwrap();
    tasks::update_task_status(&conn, "t1", "paused").unwrap();
    let task = tasks::get_task(&conn, "t1").unwrap().unwrap();
    assert_eq!(task.status, "paused");
}

#[test]
fn task_set_running() {
    let conn = setup_db();
    tasks::create_task(&conn, &make_task("t1", "active")).unwrap();
    tasks::set_task_running(&conn, "t1").unwrap();
    let task = tasks::get_task(&conn, "t1").unwrap().unwrap();
    assert_eq!(task.status, "running");
}

#[test]
fn task_delete_removes_task_and_logs() {
    let conn = setup_db();
    tasks::create_task(&conn, &make_task("t1", "active")).unwrap();
    tasks::log_task_run(&conn, "t1", 100, "success", Some("ok"), None).unwrap();

    tasks::delete_task(&conn, "t1").unwrap();
    assert!(tasks::get_task(&conn, "t1").unwrap().is_none());
    assert!(tasks::get_task_logs(&conn, "t1").unwrap().is_empty());
}

#[test]
fn task_get_due_tasks() {
    let conn = setup_db();
    // Task with past next_run (should be due)
    let mut due_task = make_task("t1", "active");
    due_task.next_run = Some("2020-01-01T00:00:00+00:00".to_string());
    tasks::create_task(&conn, &due_task).unwrap();

    // Task with future next_run (should NOT be due)
    let mut future_task = make_task("t2", "active");
    future_task.next_run = Some("2099-01-01T00:00:00+00:00".to_string());
    tasks::create_task(&conn, &future_task).unwrap();

    // Paused task with past next_run (should NOT be due)
    let mut paused = make_task("t3", "paused");
    paused.next_run = Some("2020-01-01T00:00:00+00:00".to_string());
    tasks::create_task(&conn, &paused).unwrap();

    let due = tasks::get_due_tasks(&conn).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, "t1");
}

#[test]
fn task_run_logs() {
    let conn = setup_db();
    tasks::create_task(&conn, &make_task("t1", "active")).unwrap();
    tasks::log_task_run(&conn, "t1", 150, "success", Some("done"), None).unwrap();
    tasks::log_task_run(&conn, "t1", 200, "error", None, Some("timeout")).unwrap();

    let logs = tasks::get_task_logs(&conn, "t1").unwrap();
    assert_eq!(logs.len(), 2);
}

#[test]
fn task_update_after_run() {
    let conn = setup_db();
    tasks::create_task(&conn, &make_task("t1", "active")).unwrap();
    tasks::set_task_running(&conn, "t1").unwrap();
    tasks::update_task_after_run(&conn, "t1", Some("2099-12-31T00:00:00+00:00"), Some("ok")).unwrap();

    let task = tasks::get_task(&conn, "t1").unwrap().unwrap();
    assert_eq!(task.status, "active"); // reset to active after run
    assert!(task.last_run.is_some());
    assert_eq!(task.last_result.as_deref(), Some("ok"));
    assert_eq!(task.next_run.as_deref(), Some("2099-12-31T00:00:00+00:00"));
}

// ═══════════════════════════════════════════════════════════════════════════
// NOTIFICATIONS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn notification_create_and_list() {
    let conn = setup_db();
    notifications::create_notification(&conn, "n1", "Alert", "Something happened", "warning", "agent", None).unwrap();
    notifications::create_notification(&conn, "n2", "Info", "FYI", "info", "system", None).unwrap();

    let all = notifications::get_notifications(&conn, false, None).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn notification_unread_only() {
    let conn = setup_db();
    notifications::create_notification(&conn, "n1", "A", "a", "info", "agent", None).unwrap();
    notifications::create_notification(&conn, "n2", "B", "b", "info", "agent", None).unwrap();
    notifications::mark_read(&conn, "n1").unwrap();

    let unread = notifications::get_notifications(&conn, true, None).unwrap();
    assert_eq!(unread.len(), 1);
    assert_eq!(unread[0].id, "n2");
}

#[test]
fn notification_mark_all_read() {
    let conn = setup_db();
    notifications::create_notification(&conn, "n1", "A", "a", "info", "agent", None).unwrap();
    notifications::create_notification(&conn, "n2", "B", "b", "info", "agent", None).unwrap();

    assert_eq!(notifications::unread_count(&conn).unwrap(), 2);
    notifications::mark_all_read(&conn).unwrap();
    assert_eq!(notifications::unread_count(&conn).unwrap(), 0);
}

#[test]
fn notification_delete() {
    let conn = setup_db();
    notifications::create_notification(&conn, "n1", "A", "a", "info", "agent", None).unwrap();
    notifications::delete_notification(&conn, "n1").unwrap();
    let all = notifications::get_notifications(&conn, false, None).unwrap();
    assert!(all.is_empty());
}

#[test]
fn notification_unread_count() {
    let conn = setup_db();
    notifications::create_notification(&conn, "n1", "A", "a", "info", "agent", None).unwrap();
    notifications::create_notification(&conn, "n2", "B", "b", "info", "agent", None).unwrap();
    notifications::create_notification(&conn, "n3", "C", "c", "info", "agent", None).unwrap();

    assert_eq!(notifications::unread_count(&conn).unwrap(), 3);
    notifications::mark_read(&conn, "n2").unwrap();
    assert_eq!(notifications::unread_count(&conn).unwrap(), 2);
}
