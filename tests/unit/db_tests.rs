use tempfile::tempdir;
use ccswitch::db::Db;
use rusqlite::Connection;

#[test]
fn test_db_open_and_migrate() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Db::open(&db_path).unwrap();
    // Should not panic
    db.set_setting("test_key", "test_value").unwrap();
    assert_eq!(db.get_setting("test_key"), Some("test_value".to_string()));
}

#[test]
fn test_v2_profile_migrates_to_four_models() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("ccswitch.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE profiles (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            provider_id TEXT NOT NULL DEFAULT '',
            reasoning_model TEXT NOT NULL,
            task_model TEXT NOT NULL DEFAULT '',
            is_default BOOLEAN NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            source TEXT NOT NULL DEFAULT 'user'
         );
         INSERT INTO profiles (id, name, provider_id, reasoning_model, task_model)
         VALUES ('legacy', 'Legacy', 'p1', 'reasoning', 'task');
         CREATE TABLE session_history (
             id TEXT PRIMARY KEY,
             app_type TEXT NOT NULL CHECK(app_type IN ('claude','codex')),
             project_path TEXT NOT NULL,
             profile_id TEXT,
             mode TEXT NOT NULL CHECK(mode IN ('local','proxy')),
             start_time TEXT NOT NULL,
             end_time TEXT,
             prompt_tokens INTEGER NOT NULL DEFAULT 0,
             completion_tokens INTEGER NOT NULL DEFAULT 0,
             message_count INTEGER NOT NULL DEFAULT 0,
             title TEXT,
             size_bytes INTEGER NOT NULL DEFAULT 0,
             file_mtime TEXT NOT NULL DEFAULT ''
         );
         CREATE TABLE session_log_sync (
             file_path TEXT PRIMARY KEY,
             file_mtime INTEGER NOT NULL,
             scan_type TEXT NOT NULL DEFAULT '',
             last_synced_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         PRAGMA user_version = 2;",
    ).unwrap();
    drop(conn);

    let db = Db::open(&db_path).unwrap();
    let values: (String, String, String, String) = db.conn().query_row(
        "SELECT opus_model, sonnet_model, haiku_model, subagent_model FROM profiles WHERE id='legacy'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap();
    assert_eq!(values, ("reasoning".into(), "reasoning".into(), "task".into(), "task".into()));
}
