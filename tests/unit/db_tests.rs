use ccswitch::core::models::{Profile, Source};
use ccswitch::db::sessions::SessionRecord;
use ccswitch::db::Db;
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn test_db_open_and_migrate() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Db::open(&db_path).unwrap();
    // Should not panic
    db.set_setting("test_key", "test_value").unwrap();
    assert_eq!(db.get_setting("test_key"), Some("test_value".to_string()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&db_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn rejects_database_from_a_newer_schema_version() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("future.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.pragma_update(None, "user_version", 999).unwrap();
    drop(conn);

    let error = Db::open(&db_path)
        .err()
        .expect("future schema must be rejected");
    assert!(error.to_string().contains("newer than this CCSwitch build"));
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
         CREATE TABLE usage_logs (
             app_type TEXT NOT NULL,
             message_id TEXT
         );
         CREATE UNIQUE INDEX idx_usage_msg_id ON usage_logs(message_id) WHERE message_id IS NOT NULL;
         PRAGMA user_version = 2;",
    ).unwrap();
    drop(conn);

    let db = Db::open(&db_path).unwrap();
    let values: (String, String, String, String) = db.conn().query_row(
        "SELECT opus_model, sonnet_model, haiku_model, subagent_model FROM profiles WHERE id='legacy'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap();
    assert_eq!(
        values,
        (
            "reasoning".into(),
            "reasoning".into(),
            "task".into(),
            "task".into()
        )
    );
}

#[test]
fn provider_scoped_profiles_and_app_scoped_sessions_do_not_collide() {
    let dir = tempdir().unwrap();
    let db = Db::open(&dir.path().join("ccswitch.db")).unwrap();
    for (provider, model) in [("p1", "model-1"), ("p2", "model-2")] {
        db.insert_profile(
            provider,
            &Profile {
                id: "default".into(),
                name: provider.into(),
                opus: model.into(),
                sonnet: model.into(),
                haiku: model.into(),
                subagent: model.into(),
                default: false,
                source: Source::User,
            },
        )
        .unwrap();
    }
    assert_eq!(db.get_profiles("p1").unwrap()[0].opus, "model-1");
    assert_eq!(db.get_profiles("p2").unwrap()[0].opus, "model-2");

    let session = |app: &str| SessionRecord {
        id: "same-id".into(),
        project_path: format!("/tmp/{}", app),
        profile_id: None,
        mode: if app == "codex" {
            "direct".into()
        } else {
            "local".into()
        },
        start_time: "2026-07-27 00:00:00".into(),
        end_time: None,
        prompt_tokens: 0,
        completion_tokens: 0,
        message_count: 1,
        title: Some(app.into()),
        size_bytes: 1,
        file_mtime: "2026-07-27 00:00:00".into(),
        search_text: String::new(),
    };
    db.insert_session(&session("claude"), "claude").unwrap();
    db.insert_session(&session("codex"), "codex").unwrap();
    assert_eq!(
        db.query_sessions("claude", None, None, 10).unwrap().len(),
        1
    );
    assert_eq!(db.query_sessions("codex", None, None, 10).unwrap().len(), 1);
}
