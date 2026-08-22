use super::connection::Db;
use rusqlite::params;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct UsageSummary {
    pub model: String,
    pub total_prompt: i64,
    pub total_completion: i64,
    pub total_cache_read: i64,
    pub total_cache_create: i64,
    pub request_count: i64,
}

#[derive(Debug, Clone, Default)]
pub struct SessionUsageDetails {
    pub model: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
}

pub type DailyUsage = (String, i64, i64, i64, i64);

impl Db {
    pub fn query_session_usage_details(&self, app_type: &str, session_id: &str) -> Result<SessionUsageDetails, rusqlite::Error> {
        let mut stmt = self.conn().prepare(
            "WITH RECURSIVE family(id) AS (
                 SELECT ?2
                 UNION
                 SELECT child.id
                 FROM session_history child
                 JOIN family ON child.parent_thread_id = family.id
                 WHERE child.app_type = ?1
             )
             SELECT COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0),
                    COALESCE(SUM(cache_read_tokens), 0),
                    COALESCE(SUM(cache_creation_tokens), 0),
                    COALESCE((SELECT model FROM usage_logs latest
                              WHERE latest.app_type = ?1
                                AND latest.session_id IN (SELECT id FROM family)
                              ORDER BY timestamp DESC, latest.id DESC LIMIT 1), '')
             FROM usage_logs
             WHERE app_type = ?1 AND session_id IN (SELECT id FROM family)",
        )?;
        stmt.query_row(params![app_type, session_id], |row| {
            Ok(SessionUsageDetails {
                prompt_tokens: row.get(0)?,
                completion_tokens: row.get(1)?,
                cache_read_tokens: row.get(2)?,
                cache_creation_tokens: row.get(3)?,
                model: row.get(4)?,
            })
        })
    }

    pub fn query_usage(&self, app_type: &str, range: &str) -> Result<Vec<UsageSummary>, rusqlite::Error> {
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
    pub fn query_daily_usage(&self, app_type: &str, model: &str) -> Result<Vec<DailyUsage>, rusqlite::Error> {
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
    pub fn query_session_tokens(&self, app_type: &str, session_id: &str) -> Result<(i64, i64), rusqlite::Error> {
        let mut stmt = self.conn().prepare(
            "WITH RECURSIVE family(id) AS (
                 SELECT ?2
                 UNION
                 SELECT child.id
                 FROM session_history child
                 JOIN family ON child.parent_thread_id = family.id
                 WHERE child.app_type = ?1
             )
             SELECT COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0)
             FROM usage_logs
             WHERE app_type = ?1 AND session_id IN (SELECT id FROM family)",
        )?;
        stmt.query_row(params![app_type, session_id], |row| Ok((row.get(0)?, row.get(1)?)))
    }
}
