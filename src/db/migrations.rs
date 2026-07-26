use anyhow::Context;
use rusqlite::Connection;

/// Current schema version. Increment each time we add a migration step.
pub(crate) const CURRENT_USER_VERSION: i32 = 5;

/// Apply schema migrations on the given connection.
pub(crate) fn apply_migrations(conn: &Connection) -> Result<(), anyhow::Error> {
    let version: i32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .context("read user_version")?;

    if version > CURRENT_USER_VERSION {
        anyhow::bail!(
            "Database version {} is newer than this app (max {}). \
             Please upgrade CCSwitch.",
            version,
            CURRENT_USER_VERSION
        );
    }

    if version < 1 {
        migrate_v1(conn).context("migrate v1")?;
    }
    if version < 2 {
        migrate_v2(conn).context("migrate v2")?;
    }
    if version < 3 {
        migrate_v3(conn).context("migrate v3")?;
    }
    if version < 4 {
        migrate_v4(conn).context("migrate v4")?;
    }
    if version < 5 {
        migrate_v5(conn).context("migrate v5")?;
    }

    Ok(())
}

fn migrate_v1(conn: &Connection) -> Result<(), anyhow::Error> {
    conn.execute_batch(
        "BEGIN;
         -- ── 配置层 ──
         CREATE TABLE IF NOT EXISTS providers (
             id TEXT NOT NULL,
             app_type TEXT NOT NULL CHECK(app_type IN ('claude','codex')),
             name TEXT NOT NULL,
             api_url TEXT NOT NULL,
             api_key TEXT NOT NULL DEFAULT '',
             PRIMARY KEY (id, app_type)
         );

         CREATE TABLE IF NOT EXISTS profiles (
             id TEXT PRIMARY KEY,
             name TEXT NOT NULL,
             provider_id TEXT NOT NULL DEFAULT '',
             reasoning_model TEXT NOT NULL,
             task_model TEXT NOT NULL DEFAULT '',
             is_default BOOLEAN NOT NULL DEFAULT 0,
             created_at TEXT NOT NULL DEFAULT (datetime('now'))
         );

         CREATE TABLE IF NOT EXISTS settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );

         -- ── 数据层 ──
         CREATE TABLE IF NOT EXISTS usage_logs (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             app_type TEXT NOT NULL CHECK(app_type IN ('claude','codex')),
             provider_id TEXT NOT NULL DEFAULT '',
             profile_id TEXT NOT NULL DEFAULT '',
             session_id TEXT,
             model TEXT NOT NULL,
             prompt_tokens INTEGER NOT NULL DEFAULT 0,
             completion_tokens INTEGER NOT NULL DEFAULT 0,
             cache_read_tokens INTEGER NOT NULL DEFAULT 0,
             cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
             total_tokens INTEGER NOT NULL DEFAULT 0,
             timestamp TEXT NOT NULL DEFAULT (datetime('now')),
             data_source TEXT NOT NULL DEFAULT 'import',
             message_id TEXT
         );

         CREATE INDEX IF NOT EXISTS idx_usage_app_model ON usage_logs(app_type, model, timestamp);
         CREATE INDEX IF NOT EXISTS idx_usage_session ON usage_logs(session_id);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_usage_msg_id ON usage_logs(message_id) WHERE message_id IS NOT NULL;

         CREATE TABLE IF NOT EXISTS session_history (
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

         CREATE INDEX IF NOT EXISTS idx_session_app_project ON session_history(app_type, project_path, start_time DESC);
         CREATE INDEX IF NOT EXISTS idx_session_mtime ON session_history(file_mtime DESC);

         CREATE TABLE IF NOT EXISTS proxy_request_logs (
             request_id TEXT PRIMARY KEY,
             app_type TEXT NOT NULL CHECK(app_type IN ('claude','codex')),
             provider_id TEXT NOT NULL,
             model TEXT NOT NULL,
             request_model TEXT,
             pricing_model TEXT,
             input_tokens INTEGER NOT NULL DEFAULT 0,
             output_tokens INTEGER NOT NULL DEFAULT 0,
             cache_read_tokens INTEGER NOT NULL DEFAULT 0,
             cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
             input_cost_usd TEXT NOT NULL DEFAULT '0',
             output_cost_usd TEXT NOT NULL DEFAULT '0',
             cache_read_cost_usd TEXT NOT NULL DEFAULT '0',
             cache_creation_cost_usd TEXT NOT NULL DEFAULT '0',
             total_cost_usd TEXT NOT NULL DEFAULT '0',
             latency_ms INTEGER NOT NULL,
             first_token_ms INTEGER,
             duration_ms INTEGER,
             status_code INTEGER NOT NULL,
             error_message TEXT,
             session_id TEXT,
             is_streaming INTEGER NOT NULL DEFAULT 0,
             cost_multiplier TEXT NOT NULL DEFAULT '1.0',
             created_at INTEGER NOT NULL
         );

         -- ── 追踪层 ──
         CREATE TABLE IF NOT EXISTS session_log_sync (
             file_path TEXT PRIMARY KEY,
             file_mtime INTEGER NOT NULL,
             scan_type TEXT NOT NULL DEFAULT '',
             last_synced_at TEXT NOT NULL DEFAULT (datetime('now'))
         );

         -- ── 支撑层 ──
         CREATE TABLE IF NOT EXISTS model_pricing (
             model_id TEXT PRIMARY KEY,
             display_name TEXT NOT NULL,
             input_cost_per_million REAL NOT NULL DEFAULT 0,
             output_cost_per_million REAL NOT NULL DEFAULT 0,
             cache_read_cost_per_million REAL NOT NULL DEFAULT 0,
             cache_creation_cost_per_million REAL NOT NULL DEFAULT 0
         );

         CREATE TABLE IF NOT EXISTS provider_health (
             provider_id TEXT NOT NULL,
             app_type TEXT NOT NULL,
             is_healthy BOOLEAN NOT NULL DEFAULT 1,
             consecutive_failures INTEGER NOT NULL DEFAULT 0,
             last_failure_at TEXT,
             last_error TEXT,
             PRIMARY KEY (provider_id, app_type),
             FOREIGN KEY (provider_id, app_type) REFERENCES providers(id, app_type) ON DELETE CASCADE
         );

         PRAGMA user_version = 1;
         COMMIT;",
    )?;

    tracing::info!("Migration v1 complete: 10 tables created");
    Ok(())
}

fn migrate_v2(conn: &Connection) -> Result<(), anyhow::Error> {
    conn.execute_batch(
        "BEGIN;
         -- Add source column (system = defaults.toml, user = manually added)
         ALTER TABLE providers ADD COLUMN source TEXT NOT NULL DEFAULT 'user';
         ALTER TABLE profiles ADD COLUMN source TEXT NOT NULL DEFAULT 'user';

         PRAGMA user_version = 2;
         COMMIT;",
    )?;

    tracing::info!("Migration v2 complete: source columns added");
    Ok(())
}

fn migrate_v3(conn: &Connection) -> Result<(), anyhow::Error> {
    conn.execute_batch(
        "BEGIN;
         ALTER TABLE profiles RENAME COLUMN reasoning_model TO opus_model;
         ALTER TABLE profiles RENAME COLUMN task_model TO haiku_model;
         ALTER TABLE profiles ADD COLUMN sonnet_model TEXT NOT NULL DEFAULT '';
         ALTER TABLE profiles ADD COLUMN subagent_model TEXT NOT NULL DEFAULT '';
         UPDATE profiles SET sonnet_model = opus_model WHERE sonnet_model = '';
         UPDATE profiles SET subagent_model = haiku_model WHERE subagent_model = '';

         PRAGMA user_version = 3;
         COMMIT;",
    )?;

    tracing::info!("Migration v3 complete: Claude profiles expanded to four model fields");
    Ok(())
}

fn migrate_v4(conn: &Connection) -> Result<(), anyhow::Error> {
    conn.execute_batch(
        "BEGIN;
         ALTER TABLE session_history RENAME TO session_history_v3;
         CREATE TABLE session_history (
             id TEXT PRIMARY KEY,
             app_type TEXT NOT NULL CHECK(app_type IN ('claude','codex')),
             project_path TEXT NOT NULL,
             profile_id TEXT,
             mode TEXT NOT NULL CHECK(mode IN ('local','proxy','direct')),
             start_time TEXT NOT NULL,
             end_time TEXT,
             prompt_tokens INTEGER NOT NULL DEFAULT 0,
             completion_tokens INTEGER NOT NULL DEFAULT 0,
             message_count INTEGER NOT NULL DEFAULT 0,
             title TEXT,
             size_bytes INTEGER NOT NULL DEFAULT 0,
             file_mtime TEXT NOT NULL DEFAULT ''
         );
         INSERT INTO session_history
             SELECT id, app_type, project_path, profile_id, mode, start_time, end_time,
                    prompt_tokens, completion_tokens, message_count, title, size_bytes, file_mtime
             FROM session_history_v3;
         DROP TABLE session_history_v3;
         CREATE INDEX idx_session_app_project ON session_history(app_type, project_path, start_time DESC);
         CREATE INDEX idx_session_mtime ON session_history(file_mtime DESC);

         PRAGMA user_version = 4;
         COMMIT;",
    )?;

    tracing::info!("Migration v4 complete: Codex direct session mode added");
    Ok(())
}

fn migrate_v5(conn: &Connection) -> Result<(), anyhow::Error> {
    conn.execute_batch(
        "BEGIN;
         ALTER TABLE session_log_sync RENAME TO session_log_sync_v4;
         CREATE TABLE session_log_sync (
             file_path TEXT NOT NULL,
             file_mtime INTEGER NOT NULL,
             scan_type TEXT NOT NULL CHECK(scan_type IN ('session','usage')),
             last_synced_at TEXT NOT NULL DEFAULT (datetime('now')),
             PRIMARY KEY (file_path, scan_type)
         );
         INSERT OR IGNORE INTO session_log_sync (file_path, file_mtime, scan_type, last_synced_at)
             SELECT file_path, file_mtime, 'session', last_synced_at
             FROM session_log_sync_v4 WHERE scan_type IN ('session','both');
         INSERT OR IGNORE INTO session_log_sync (file_path, file_mtime, scan_type, last_synced_at)
             SELECT file_path, file_mtime, 'usage', last_synced_at
             FROM session_log_sync_v4 WHERE scan_type IN ('usage','both');
         DROP TABLE session_log_sync_v4;

         PRAGMA user_version = 5;
         COMMIT;",
    )?;

    tracing::info!("Migration v5 complete: independent session and usage file indexes");
    Ok(())
}
