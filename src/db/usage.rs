use super::connection::Db;
use rusqlite::params;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct UsageSummary {
    pub model: String,
    pub total_prompt: i64,
    pub total_completion: i64,
    pub total_cache_read: i64,
    pub total_cache_create: i64,
    pub request_count: i64,
}

pub type DailyUsage = (String, i64, i64, i64, i64);

/// Progress event sent from the background scan thread to the TUI
#[derive(Debug, Clone)]
pub enum ScanEvent {
    /// A batch of parsed records from one file, ready for DB insert
    Batch {
        #[allow(dead_code)]
        app_type: String,
        sid: String,
        file_path: PathBuf,
        records: Vec<UsageRecord>,
    },
    /// Progress update (files done / files total, cumulative records)
    Progress {
        files_done: usize,
        files_total: usize,
        records: usize,
    },
    /// All files parsed.
    Done {},
}

/// Lightweight parser for assistant message usage data in JSONL files
/// Parsed usage record awaiting DB insert
#[derive(Debug, Clone)]
pub struct UsageRecord {
    pub msg_id: String,
    pub model: String,
    pub date: String,
    pub input: i64,
    pub output: i64,
    pub cr: i64,
    pub cc: i64,
}

/// Scan context prepared on main thread (DB queries only, fast).
#[derive(Debug, Clone)]
pub struct ScanContext {
    pub file_index: std::collections::HashMap<String, i64>, // file_path → mtime
}

impl Db {
    pub fn query_usage(
        &self,
        app_type: &str,
        range: &str,
    ) -> Result<Vec<UsageSummary>, rusqlite::Error> {
        let date_filter = match range {
            "day" => "date(timestamp) = date('now')",
            "week" => "date(timestamp) >= date('now', '-6 days')",
            "month" => "date(timestamp) >= date('now', '-30 days')",
            _ => "1=1",
        };
        let sql = format!(
            "SELECT model, SUM(prompt_tokens), SUM(completion_tokens), SUM(cache_read_tokens), SUM(cache_creation_tokens), COUNT(*)
             FROM usage_logs WHERE app_type = ?1 AND {} GROUP BY model ORDER BY MAX(timestamp) DESC",
            date_filter
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![app_type], |row| {
            Ok(UsageSummary {
                model: row.get(0)?,
                total_prompt: row.get(1)?,
                total_completion: row.get(2)?,
                total_cache_read: row.get(3)?,
                total_cache_create: row.get(4)?,
                request_count: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    /// Query per-day usage breakdown for a specific model
    pub fn query_daily_usage(
        &self,
        app_type: &str,
        model: &str,
    ) -> Result<Vec<DailyUsage>, rusqlite::Error> {
        let sql = "SELECT date(timestamp) as day,
                          SUM(prompt_tokens), SUM(completion_tokens),
                          SUM(cache_read_tokens), SUM(cache_creation_tokens)
                   FROM usage_logs
                   WHERE app_type = ?1 AND model = ?2
                     AND date(timestamp) >= date('now', '-6 days')
                   GROUP BY day ORDER BY day";
        let mut stmt = self.conn().prepare(sql)?;
        let rows = stmt.query_map(params![app_type, model], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        rows.collect()
    }

    /// Query token usage for a specific session (by session ID)
    pub fn query_session_tokens(
        &self,
        app_type: &str,
        session_id: &str,
    ) -> Result<(i64, i64), rusqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0)
             FROM usage_logs WHERE app_type = ?1 AND session_id = ?2",
        )?;
        stmt.query_row(params![app_type, session_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
    }

    /// Prepare scan context: load the file sync index from session_log_sync.
    /// Called from main thread. Fast — no filesystem walk.
    pub fn prepare_scan_context(&self) -> Result<ScanContext, anyhow::Error> {
        // Cleanup synthetic entries
        if let Err(e) = self
            .conn()
            .execute("DELETE FROM usage_logs WHERE model = '<synthetic>'", [])
        {
            tracing::warn!("synthetic cleanup failed: {}", e);
        }

        // Load stored file index from session_log_sync
        let file_index: std::collections::HashMap<String, i64> = {
            let mut stmt = self.conn().prepare(
                "SELECT file_path, file_mtime FROM session_log_sync WHERE scan_type = 'usage'",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            rows.filter_map(|r| r.ok()).collect()
        };

        Ok(ScanContext { file_index })
    }

    /// Insert a batch of parsed records into usage_logs (single transaction for speed).
    pub fn insert_usage_batch(
        &self,
        app_type: &str,
        sid: &str,
        file_path: &PathBuf,
        records: &[UsageRecord],
    ) -> Result<usize, anyhow::Error> {
        let mut imported = 0usize;
        let result: Result<usize, anyhow::Error> = (|| {
            self.conn().execute("BEGIN", [])?;
            self.conn().execute(
                "DELETE FROM usage_logs WHERE app_type = ?1 AND session_id = ?2 AND data_source = 'import'",
                params![app_type, sid],
            )?;
            for r in records {
                let inserted = self.conn().execute(
                    "INSERT OR IGNORE INTO usage_logs (app_type, model, provider_id, profile_id, session_id, message_id,
                     prompt_tokens, completion_tokens, cache_read_tokens, cache_creation_tokens, total_tokens, timestamp, data_source)
                     VALUES (?1, ?2, '', '', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'import')",
                    params![
                        app_type,
                        r.model,
                        sid,
                        r.msg_id,
                        r.input,
                        r.output,
                        r.cr,
                        r.cc,
                        r.input
                            .saturating_add(r.output)
                            .saturating_add(r.cr)
                            .saturating_add(r.cc),
                        r.date
                    ],
                )?;
                imported += inserted;
            }
            if let Some(mtime) = file_mtime(file_path) {
                let now_iso = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let file_path_str = file_path.to_string_lossy().to_string();
                self.conn().execute(
                    "INSERT INTO session_log_sync (file_path, file_mtime, scan_type, last_synced_at)
                     VALUES (?1, ?2, 'usage', ?3)
                     ON CONFLICT(file_path, scan_type) DO UPDATE SET
                        file_mtime=excluded.file_mtime,
                        last_synced_at=excluded.last_synced_at",
                    params![file_path_str, mtime, now_iso],
                )?;
            }
            self.conn().execute("COMMIT", [])?;
            Ok(imported)
        })();
        if result.is_err() {
            let _ = self.conn().execute("ROLLBACK", []);
        }
        result
    }
}

fn file_mtime(path: &PathBuf) -> Option<i64> {
    crate::core::import::file_mtime(path)
}
