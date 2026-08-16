use super::connection::Db;
use anyhow::Context;
use rusqlite::params;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SessionRecord {
    pub id: String,
    pub project_path: String,
    pub profile_id: Option<String>,
    /// Set only for Codex internal child threads. Explicit `/fork` sessions
    /// use different metadata and remain independent history entries.
    pub parent_thread_id: Option<String>,
    pub mode: String,
    pub start_time: String,
    pub end_time: Option<String>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub message_count: i64,
    pub title: Option<String>,
    pub size_bytes: i64,
    /// JSONL file modification time (ISO string) — used for relative-time display
    pub file_mtime: String,
    /// Pre-computed lowercase search text (title + project_path)
    #[serde(skip)]
    pub search_text: String,
}

impl Db {
    pub fn query_session(&self, app_type: &str, id: &str) -> Result<Option<SessionRecord>, rusqlite::Error> {
        let mut stmt = self.conn().prepare(
            "WITH RECURSIVE family(id) AS (
                 SELECT ?2
                 UNION
                 SELECT child.id
                 FROM session_history child
                 JOIN family ON child.parent_thread_id = family.id
                 WHERE child.app_type = ?1
             )
             SELECT root.id, root.project_path, root.profile_id, root.parent_thread_id,
                    root.mode, root.start_time, root.end_time,
                    root.prompt_tokens, root.completion_tokens,
                    COALESCE((SELECT SUM(member.message_count)
                              FROM session_history member
                              JOIN family ON family.id = member.id
                              WHERE member.app_type = ?1), root.message_count),
                    root.title, root.size_bytes, root.file_mtime
             FROM session_history root WHERE root.app_type = ?1 AND root.id = ?2",
        )?;
        let mut rows = stmt.query_map(params![app_type, id], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                project_path: row.get(1)?,
                profile_id: row.get(2)?,
                parent_thread_id: row.get(3)?,
                mode: row.get(4)?,
                start_time: row.get(5)?,
                end_time: row.get(6)?,
                prompt_tokens: row.get(7)?,
                completion_tokens: row.get(8)?,
                message_count: row.get(9)?,
                title: row.get(10)?,
                size_bytes: row.get::<_, i64>(11).unwrap_or(0),
                file_mtime: row.get::<_, String>(12).unwrap_or_default(),
                search_text: String::new(),
            })
        })?;
        rows.next().transpose()
    }

    pub fn insert_session(&self, s: &SessionRecord, app_type: &str) -> Result<(), rusqlite::Error> {
        self.conn().execute(
            "INSERT INTO session_history (id, app_type, project_path, profile_id, parent_thread_id, mode, start_time, end_time, prompt_tokens, completion_tokens, message_count, title, size_bytes, file_mtime)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(id, app_type) DO UPDATE SET
                project_path=excluded.project_path, profile_id=excluded.profile_id,
                parent_thread_id=excluded.parent_thread_id,
                mode=excluded.mode, start_time=excluded.start_time, end_time=excluded.end_time,
                prompt_tokens=excluded.prompt_tokens, completion_tokens=excluded.completion_tokens,
                message_count=excluded.message_count, title=excluded.title,
                size_bytes=excluded.size_bytes, file_mtime=excluded.file_mtime",
            params![s.id, app_type, s.project_path, s.profile_id, s.parent_thread_id, s.mode, s.start_time, s.end_time, s.prompt_tokens, s.completion_tokens, s.message_count, s.title, s.size_bytes, s.file_mtime],
        )?;
        Ok(())
    }

    /// Delete a session record, its usage logs, and the on-disk JSONL file.
    pub fn delete_session(&self, id: &str, app_type: &str) -> Result<(), anyhow::Error> {
        let transaction = self.conn().unchecked_transaction()?;
        transaction.execute("DELETE FROM usage_logs WHERE session_id = ?1 AND app_type = ?2", params![id, app_type])?;
        transaction.execute("DELETE FROM session_history WHERE id = ?1 AND app_type = ?2", params![id, app_type])?;

        // Delete the actual JSONL file from disk
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let projects_dir = if app_type == "codex" {
            std::path::PathBuf::from(&home).join(".codex/sessions")
        } else {
            std::path::PathBuf::from(&home).join(".claude/projects")
        };
        let file_name = format!("{}.jsonl", id);
        if let Some(file_path) = Self::find_session_file(&projects_dir, &file_name, app_type == "codex") {
            std::fs::remove_file(&file_path).with_context(|| format!("Failed to delete session file {}", file_path.display()))?;
        }

        transaction.commit()?;
        Ok(())
    }

    /// Find a session JSONL file by name under the projects directory (recursive).
    fn find_session_file(dir: &std::path::Path, file_name: &str, allow_codex_suffix: bool) -> Option<std::path::PathBuf> {
        Self::find_session_file_impl(dir, file_name, allow_codex_suffix, 10)
    }

    fn find_session_file_impl(dir: &std::path::Path, file_name: &str, allow_codex_suffix: bool, depth: usize) -> Option<std::path::PathBuf> {
        if depth == 0 {
            return None;
        }
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip symlinks to avoid cycles
            if path.is_symlink() {
                continue;
            }
            if path.is_dir() {
                if let Some(found) = Self::find_session_file_impl(&path, file_name, allow_codex_suffix, depth - 1) {
                    return Some(found);
                }
            } else if path
                .file_name()
                .is_some_and(|name| name == file_name || (allow_codex_suffix && name.to_string_lossy().ends_with(&format!("-{}", file_name))))
            {
                return Some(path);
            }
        }
        None
    }

    pub fn query_sessions(&self, app_type: &str, project: Option<&str>, search: Option<&str>, limit: usize) -> Result<Vec<SessionRecord>, rusqlite::Error> {
        let mut sql = String::from(
            "WITH RECURSIVE session_tree(root_id, id) AS (
                 SELECT id, id FROM session_history
                 WHERE app_type = ?1 AND COALESCE(parent_thread_id, '') = ''
                 UNION
                 SELECT tree.root_id, child.id
                 FROM session_tree tree
                 JOIN session_history child
                   ON child.app_type = ?1 AND child.parent_thread_id = tree.id
             ),
             message_totals(root_id, message_count) AS (
                 SELECT tree.root_id, SUM(session.message_count)
                 FROM session_tree tree
                 JOIN session_history session
                   ON session.app_type = ?1 AND session.id = tree.id
                 GROUP BY tree.root_id
             )
             SELECT root.id, root.project_path, root.profile_id, root.parent_thread_id,
                    root.mode, root.start_time, root.end_time,
                    root.prompt_tokens, root.completion_tokens,
                    COALESCE(totals.message_count, root.message_count),
                    root.title, root.size_bytes, root.file_mtime
             FROM session_history root
             LEFT JOIN message_totals totals ON totals.root_id = root.id
             WHERE root.app_type = ?1 AND COALESCE(root.parent_thread_id, '') = ''",
        );
        let mut param_values: Vec<String> = vec![app_type.to_string()];

        if let Some(p) = project {
            param_values.push(format!("%{}%", p));
            let idx = param_values.len();
            sql.push_str(&format!(" AND project_path LIKE ?{}", idx));
        }
        if let Some(s) = search {
            let pattern = format!("%{}%", s);
            param_values.push(pattern.clone());
            param_values.push(pattern);
            let idx1 = param_values.len() - 1;
            let idx2 = param_values.len();
            sql.push_str(&format!(" AND (title LIKE ?{} OR id LIKE ?{})", idx1, idx2));
        }
        sql.push_str(" ORDER BY file_mtime DESC, start_time DESC LIMIT ?");
        let limit_str = limit.to_string();
        param_values.push(limit_str);

        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(param_values.iter().map(|s| s.as_str())), |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                project_path: row.get(1)?,
                profile_id: row.get(2)?,
                parent_thread_id: row.get(3)?,
                mode: row.get(4)?,
                start_time: row.get(5)?,
                end_time: row.get(6)?,
                prompt_tokens: row.get(7)?,
                completion_tokens: row.get(8)?,
                message_count: row.get(9)?,
                title: row.get(10)?,
                size_bytes: row.get::<_, i64>(11).unwrap_or(0),
                file_mtime: row.get::<_, String>(12).unwrap_or_default(),
                search_text: String::new(), // populated below
            })
        })?;
        let mut rows: Vec<SessionRecord> = rows.collect::<Result<Vec<_>, _>>()?;
        for s in &mut rows {
            if s.search_text.is_empty() {
                s.search_text = format!("{} {}", s.title.as_deref().unwrap_or(""), s.project_path).to_lowercase();
            }
        }
        Ok(rows)
    }

    /// Query the unified Claude and Codex history index for the session workbench.
    #[allow(dead_code)]
    pub fn query_all_sessions(&self, search: Option<&str>, limit: usize) -> Result<Vec<(String, SessionRecord)>, rusqlite::Error> {
        let pattern = search.map(|value| format!("%{}%", value));
        let mut stmt = self.conn().prepare(
            "WITH RECURSIVE session_tree(root_app, root_id, app_type, id) AS (
                 SELECT app_type, id, app_type, id
                 FROM session_history
                 WHERE COALESCE(parent_thread_id, '') = ''
                 UNION
                 SELECT tree.root_app, tree.root_id, child.app_type, child.id
                 FROM session_tree tree
                 JOIN session_history child
                   ON child.app_type = tree.app_type AND child.parent_thread_id = tree.id
             ),
             message_totals(root_app, root_id, message_count) AS (
                 SELECT tree.root_app, tree.root_id, SUM(session.message_count)
                 FROM session_tree tree
                 JOIN session_history session
                   ON session.app_type = tree.app_type AND session.id = tree.id
                 GROUP BY tree.root_app, tree.root_id
             )
             SELECT root.id, root.app_type, root.project_path, root.profile_id,
                    root.parent_thread_id, root.mode, root.start_time, root.end_time,
                    root.prompt_tokens, root.completion_tokens,
                    COALESCE(totals.message_count, root.message_count),
                    root.title, root.size_bytes, root.file_mtime
             FROM session_history root
             LEFT JOIN message_totals totals
               ON totals.root_app = root.app_type AND totals.root_id = root.id
             WHERE COALESCE(root.parent_thread_id, '') = ''
               AND (?1 IS NULL OR root.title LIKE ?1 OR root.project_path LIKE ?1 OR root.id LIKE ?1)
             ORDER BY root.file_mtime DESC, root.start_time DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![pattern, limit as i64], |row| {
            let app_type: String = row.get(1)?;
            let title: Option<String> = row.get(11)?;
            let project_path: String = row.get(2)?;
            Ok((
                app_type,
                SessionRecord {
                    id: row.get(0)?,
                    project_path: project_path.clone(),
                    profile_id: row.get(3)?,
                    parent_thread_id: row.get(4)?,
                    mode: row.get(5)?,
                    start_time: row.get(6)?,
                    end_time: row.get(7)?,
                    prompt_tokens: row.get(8)?,
                    completion_tokens: row.get(9)?,
                    message_count: row.get(10)?,
                    title,
                    size_bytes: row.get::<_, i64>(12).unwrap_or(0),
                    file_mtime: row.get::<_, String>(13).unwrap_or_default(),
                    search_text: String::new(),
                },
            ))
        })?;
        let mut sessions = rows.collect::<Result<Vec<_>, _>>()?;
        for (_, session) in &mut sessions {
            session.search_text = format!("{} {}", session.title.as_deref().unwrap_or(""), session.project_path).to_lowercase();
        }
        Ok(sessions)
    }
}
