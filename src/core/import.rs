//! Session and usage data import from Claude Code / Codex CLI JSONL files.
//!
//! These functions read JSONL files from the filesystem, parse them, and write
//! results to the database. Separated from the `db` module to keep the DB
//! layer focused on CRUD operations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MAX_USAGE_TOKENS: i64 = 1_000_000_000_000;
const CLAUDE_IMPORT_REVISION_KEY: &str = "claude_import_revision";
const CLAUDE_IMPORT_REVISION: &str = "1";
const CODEX_IMPORT_REVISION_KEY: &str = "codex_import_revision";
const CODEX_IMPORT_REVISION: &str = "2";

use serde::Deserialize;

use crate::db::connection::Db;
use crate::db::sessions::SessionRecord;
use crate::db::usage::{ScanContext, ScanEvent, UsageRecord};

// ── Session Import ───────────────────────────────────────────────

/// A line from a Claude Code session JSONL file
#[derive(Debug, Deserialize)]
struct JsonlLine {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    timestamp: Option<serde_json::Value>,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    msg_type: Option<String>,
    #[allow(dead_code)]
    message: Option<MessageContent>,
    #[serde(rename = "customTitle")]
    custom_title: Option<String>,
    #[serde(rename = "aiTitle")]
    ai_title: Option<String>,
    #[serde(rename = "lastPrompt")]
    last_prompt: Option<String>,
    #[allow(dead_code)]
    #[serde(rename = "isMeta")]
    is_meta: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct MessageContent {
    content: Option<serde_json::Value>,
    #[allow(dead_code)]
    role: Option<String>,
    #[serde(default)]
    model: Option<String>,
    /// Proxy mode marker injected by CCSwitch
    #[serde(default)]
    ccs_proxy: Option<bool>,
}

fn claude_projects_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".claude").join("projects")
}

fn claude_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".claude.json")
}

fn codex_sessions_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".codex").join("sessions")
}

fn codex_session_index_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".codex").join("session_index.jsonl")
}

enum TitleField {
    Custom(String),
    Ai(String),
    LastPrompt(String),
}

fn parse_title_only(line: &str) -> Option<TitleField> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if let Some(title) = value.get("customTitle").and_then(serde_json::Value::as_str) {
        if !title.is_empty() {
            return Some(TitleField::Custom(title.to_string()));
        }
    }
    if let Some(title) = value.get("aiTitle").and_then(serde_json::Value::as_str) {
        if !title.is_empty() {
            return Some(TitleField::Ai(title.to_string()));
        }
    }
    if let Some(title) = value.get("lastPrompt").and_then(serde_json::Value::as_str) {
        if !title.is_empty() {
            return Some(TitleField::LastPrompt(title.to_string()));
        }
    }
    None
}

fn truncate_title(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 40 {
        format!("{}...", s.chars().take(37).collect::<String>())
    } else {
        s.to_string()
    }
}

fn parse_timestamp(val: &serde_json::Value) -> Option<i64> {
    match val {
        serde_json::Value::Number(n) => {
            let ts = n.as_i64().or_else(|| n.as_u64().and_then(|value| i64::try_from(value).ok()))?;
            if ts <= 0 {
                return None;
            }
            Some(if ts > 1_000_000_000_000 { ts } else { ts.checked_mul(1000)? })
        }
        serde_json::Value::String(s) => chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.timestamp_millis()),
        _ => None,
    }
}

fn extract_command(text: &str) -> Option<String> {
    let name = text.split("<command-name>").nth(1)?.split("</command-name>").next()?.trim().to_string();
    let args = text
        .split("<command-args>")
        .nth(1)
        .and_then(|s| s.split("</command-args>").next())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    match args {
        Some(a) => Some(format!("{} {}", name, a)),
        None => Some(name),
    }
}

fn ts_to_iso(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    let nanos = ((ts_ms % 1000) * 1_000_000) as u32;
    match chrono::TimeZone::timestamp_opt(&chrono::Utc, secs, nanos) {
        chrono::offset::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        _ => String::new(),
    }
}

/// Import with progress callback. Incremental: only processes files whose
/// mtime differs from the stored index in session_log_sync.
pub fn import_claude_sessions_with_progress(db: &Db, on_progress: impl Fn(usize, usize, usize)) -> Result<usize, anyhow::Error> {
    let projects_dir = claude_projects_dir();
    let mut file_index: HashMap<String, i64> = {
        let mut stmt = db.conn().prepare("SELECT file_path, file_mtime FROM session_log_sync WHERE scan_type = 'session'")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    cleanup_removed_session_files(db, &projects_dir, "claude", &file_index)?;
    if prepare_claude_import_revision(db, &projects_dir)? {
        file_index.retain(|path, _| !Path::new(path).starts_with(&projects_dir));
    }

    let jsonl_files = collect_jsonl_files(&projects_dir);
    let project_paths = load_claude_project_paths(&claude_config_path());
    let total = jsonl_files.len();
    let mut imported = 0usize;
    let mut updated = 0usize;
    let mut last_report = 0usize;
    let now_iso = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    const APP_TYPE: &str = "claude";

    for (idx, path) in jsonl_files.iter().enumerate() {
        let sid = path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
        if sid.is_empty() || sid.starts_with("agent-") {
            continue;
        }

        let current_mtime = file_mtime_secs(path);
        let file_path_str = path.to_string_lossy().to_string();
        if let Some(&stored_mtime) = file_index.get(&file_path_str) {
            if stored_mtime == current_mtime {
                continue;
            }
        }

        match parse_session_file(path, &projects_dir, &project_paths) {
            Ok(Some(record)) => {
                db.insert_session(&record, APP_TYPE)?;
                if file_index.contains_key(&file_path_str) {
                    updated += 1;
                } else {
                    imported += 1;
                }
                db.conn().execute(
                    "INSERT INTO session_log_sync (file_path, file_mtime, scan_type, last_synced_at)
                     VALUES (?1, ?2, 'session', ?3)
                     ON CONFLICT(file_path, scan_type) DO UPDATE SET
                        file_mtime=excluded.file_mtime,
                        last_synced_at=excluded.last_synced_at",
                    rusqlite::params![file_path_str, current_mtime, now_iso],
                )?;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("Failed to parse session file {:?}: {}", path, e);
            }
        }

        let files_done = idx + 1;
        if files_done - last_report >= 5 || files_done == total {
            on_progress(files_done, total, imported + updated);
            last_report = files_done;
        }
    }

    Ok(imported + updated)
}

pub fn import_claude_sessions(db: &Db) -> Result<usize, anyhow::Error> {
    import_claude_sessions_with_progress(db, |_, _, _| {})
}

fn claude_project_directory_name(path: &str) -> String {
    path.chars().map(|character| if matches!(character, '/' | '\\' | ':') { '-' } else { character }).collect()
}

fn load_claude_project_paths(config_path: &Path) -> HashMap<String, PathBuf> {
    let content = match std::fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(_) => return HashMap::new(),
    };
    let config: serde_json::Value = match serde_json::from_str(&content) {
        Ok(config) => config,
        Err(_) => return HashMap::new(),
    };
    let Some(projects) = config.get("projects").and_then(serde_json::Value::as_object) else {
        return HashMap::new();
    };

    let mut candidates = HashMap::<String, Option<PathBuf>>::new();
    for project_path in projects.keys().map(PathBuf::from).filter(|path| path.is_dir()) {
        let directory_name = claude_project_directory_name(&project_path.to_string_lossy());
        candidates
            .entry(directory_name)
            .and_modify(|candidate| {
                if candidate.as_ref() != Some(&project_path) {
                    *candidate = None;
                }
            })
            .or_insert(Some(project_path));
    }

    candidates.into_iter().filter_map(|(directory, path)| path.map(|path| (directory, path))).collect()
}

fn project_path_for_claude_session<'a>(path: &Path, projects_dir: &Path, project_paths: &'a HashMap<String, PathBuf>) -> Option<&'a PathBuf> {
    let directory = path.strip_prefix(projects_dir).ok()?.components().next()?.as_os_str().to_str()?;
    project_paths.get(directory)
}

#[derive(Debug, Deserialize)]
struct CodexLine {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    line_type: String,
    #[serde(default)]
    payload: serde_json::Value,
}

/// Incrementally import Codex session transcripts from ~/.codex/sessions.
pub fn import_codex_sessions(db: &Db) -> Result<usize, anyhow::Error> {
    let sessions_dir = codex_sessions_dir();
    prepare_codex_import_revision(db, &sessions_dir)?;
    let session_index = load_codex_session_index(&codex_session_index_path());
    let file_index: HashMap<String, i64> = {
        let mut stmt = db.conn().prepare("SELECT file_path, file_mtime FROM session_log_sync WHERE scan_type = 'session'")?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
        rows.filter_map(|row| row.ok()).collect()
    };
    cleanup_removed_session_files(db, &sessions_dir, "codex", &file_index)?;

    let mut imported = 0usize;
    let now_iso = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    for path in collect_jsonl_files(&sessions_dir) {
        let path_text = path.to_string_lossy().to_string();
        let mtime = file_mtime_secs(&path);
        if file_index.get(&path_text) == Some(&mtime) {
            continue;
        }
        if let Some(record) = parse_codex_session_file(&path)? {
            db.insert_session(&record, "codex")?;
            db.conn().execute(
                "INSERT INTO session_log_sync (file_path, file_mtime, scan_type, last_synced_at)
                 VALUES (?1, ?2, 'session', ?3)
                 ON CONFLICT(file_path, scan_type) DO UPDATE SET
                    file_mtime=excluded.file_mtime,
                    last_synced_at=excluded.last_synced_at",
                rusqlite::params![path_text, mtime, now_iso],
            )?;
            imported += 1;
        }
    }
    apply_codex_session_index(db, &session_index)?;
    Ok(imported)
}

/// Invalidate Codex import state when rollout parsing semantics change.
///
/// Session and usage scans have independent indexes, so both need to be
/// cleared. Imported usage is also removed because older versions may have
/// attributed fork usage to the parent session ID.
fn prepare_codex_import_revision(db: &Db, sessions_dir: &std::path::Path) -> Result<(), anyhow::Error> {
    if db.get_setting(CODEX_IMPORT_REVISION_KEY).as_deref() == Some(CODEX_IMPORT_REVISION) {
        return Ok(());
    }

    let codex_sync_paths = {
        let mut statement = db.conn().prepare("SELECT DISTINCT file_path FROM session_log_sync")?;
        let paths = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        paths.into_iter().filter(|path| std::path::Path::new(path).starts_with(sessions_dir)).collect::<Vec<_>>()
    };

    let transaction = db.conn().unchecked_transaction()?;
    transaction.execute("DELETE FROM usage_logs WHERE app_type = 'codex' AND data_source = 'import'", [])?;
    for path in codex_sync_paths {
        transaction.execute("DELETE FROM session_log_sync WHERE file_path = ?1", [path])?;
    }
    transaction.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        rusqlite::params![CODEX_IMPORT_REVISION_KEY, CODEX_IMPORT_REVISION],
    )?;
    transaction.commit()?;
    Ok(())
}

fn prepare_claude_import_revision(db: &Db, projects_dir: &Path) -> Result<bool, anyhow::Error> {
    if db.get_setting(CLAUDE_IMPORT_REVISION_KEY).as_deref() == Some(CLAUDE_IMPORT_REVISION) {
        return Ok(false);
    }

    let claude_sync_paths = {
        let mut statement = db.conn().prepare("SELECT file_path FROM session_log_sync WHERE scan_type = 'session'")?;
        let paths = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        paths.into_iter().filter(|path| Path::new(path).starts_with(projects_dir)).collect::<Vec<_>>()
    };

    let transaction = db.conn().unchecked_transaction()?;
    for path in claude_sync_paths {
        transaction.execute("DELETE FROM session_log_sync WHERE file_path = ?1 AND scan_type = 'session'", [path])?;
    }
    transaction.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        rusqlite::params![CLAUDE_IMPORT_REVISION_KEY, CLAUDE_IMPORT_REVISION],
    )?;
    transaction.commit()?;
    Ok(true)
}

fn cleanup_removed_session_files(db: &Db, root: &std::path::Path, app_type: &str, file_index: &HashMap<String, i64>) -> Result<usize, anyhow::Error> {
    let stale_paths = file_index
        .keys()
        .map(PathBuf::from)
        .filter(|path| path.starts_with(root) && !path.exists())
        .collect::<Vec<_>>();
    if stale_paths.is_empty() {
        return Ok(0);
    }

    let codex_ids = if app_type == "codex" {
        let mut statement = db.conn().prepare("SELECT id FROM session_history WHERE app_type = 'codex'")?;
        let ids = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        ids
    } else {
        Vec::new()
    };

    let transaction = db.conn().unchecked_transaction()?;
    let mut removed = 0usize;
    for path in stale_paths {
        let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
        let session_id = if app_type == "codex" {
            codex_ids
                .iter()
                .find(|id| file_name == format!("{}.jsonl", id) || file_name.ends_with(&format!("-{}.jsonl", id)))
                .cloned()
        } else {
            path.file_stem().and_then(|name| name.to_str()).map(str::to_owned)
        };

        if let Some(session_id) = session_id.as_deref() {
            transaction.execute("DELETE FROM usage_logs WHERE app_type = ?1 AND session_id = ?2", rusqlite::params![app_type, session_id])?;
            removed += transaction.execute("DELETE FROM session_history WHERE app_type = ?1 AND id = ?2", rusqlite::params![app_type, session_id])?;
        }
        transaction.execute("DELETE FROM session_log_sync WHERE file_path = ?1", [path.to_string_lossy().as_ref()])?;
    }
    transaction.commit()?;
    Ok(removed)
}

#[derive(Debug, Deserialize)]
struct CodexSessionIndexEntry {
    id: String,
    thread_name: String,
    #[allow(dead_code)]
    updated_at: Option<String>,
}

fn load_codex_session_index(path: &std::path::Path) -> HashMap<String, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return HashMap::new(),
    };
    content
        .lines()
        .filter_map(|line| {
            let entry: CodexSessionIndexEntry = serde_json::from_str(line).ok()?;
            let title = entry.thread_name.trim();
            if entry.id.is_empty() || title.is_empty() {
                return None;
            }
            Some((entry.id, truncate_title(title)))
        })
        .collect()
}

fn apply_codex_session_index(db: &Db, index: &HashMap<String, String>) -> Result<(), rusqlite::Error> {
    for (id, title) in index {
        db.conn()
            .execute("UPDATE session_history SET title = ?1 WHERE id = ?2 AND app_type = 'codex'", rusqlite::params![title, id])?;
    }
    Ok(())
}

/// Incrementally refresh native session titles without re-importing Codex rollouts.
#[allow(dead_code)]
pub fn refresh_session_titles(db: &Db) -> Result<(), anyhow::Error> {
    import_claude_sessions(db)?;
    let index = load_codex_session_index(&codex_session_index_path());
    apply_codex_session_index(db, &index)?;
    Ok(())
}

fn parse_codex_session_file(path: &PathBuf) -> Result<Option<SessionRecord>, anyhow::Error> {
    let metadata = std::fs::metadata(path)?;
    let content = std::fs::read_to_string(path)?;
    let mut session_id = String::new();
    let mut cwd = String::new();
    let mut provider = String::new();
    let mut start_time = String::new();
    let mut end_time = String::new();
    let mut title: Option<String> = None;
    let mut message_count = 0i64;

    for raw in content.lines() {
        let line: CodexLine = match serde_json::from_str(raw) {
            Ok(line) => line,
            Err(_) => continue,
        };
        if let Some(normalized) = line.timestamp.as_deref().and_then(normalize_session_timestamp) {
            if start_time.is_empty() {
                start_time = normalized.clone();
            }
            end_time = normalized;
        }
        match line.line_type.as_str() {
            "session_meta" => {
                let canonical_id = line
                    .payload
                    .get("id")
                    .or_else(|| line.payload.get("session_id"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                // A forked rollout can contain a second session_meta copied
                // from its parent. The first valid metadata record identifies
                // the rollout; later records must not overwrite it.
                if !session_id.is_empty() || canonical_id.is_empty() {
                    continue;
                }
                session_id = canonical_id.to_string();
                cwd = line.payload.get("cwd").and_then(serde_json::Value::as_str).unwrap_or("").to_string();
                provider = line.payload.get("model_provider").and_then(serde_json::Value::as_str).unwrap_or("").to_string();
                if let Some(timestamp) = line.payload.get("timestamp").and_then(serde_json::Value::as_str) {
                    if let Some(normalized) = normalize_session_timestamp(timestamp) {
                        start_time = normalized;
                    }
                }
            }
            "event_msg" => {
                let event_type = line.payload.get("type").and_then(serde_json::Value::as_str).unwrap_or("");
                if matches!(event_type, "user_message" | "agent_message") {
                    message_count += 1;
                }
                if title.is_none() && event_type == "user_message" {
                    title = line.payload.get("message").and_then(serde_json::Value::as_str).map(truncate_title);
                }
            }
            _ => {}
        }
    }
    if session_id.is_empty() {
        return Ok(None);
    }
    let project_name = std::path::Path::new(&cwd)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "Codex session".into());
    let title = title.unwrap_or(project_name);
    let file_mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| ts_to_iso(duration.as_millis() as i64))
        .unwrap_or_default();
    Ok(Some(SessionRecord {
        id: session_id,
        project_path: cwd.clone(),
        profile_id: if provider.is_empty() { None } else { Some(provider) },
        mode: "direct".into(),
        start_time,
        end_time: if end_time.is_empty() { None } else { Some(end_time) },
        prompt_tokens: 0,
        completion_tokens: 0,
        message_count,
        search_text: format!("{} {}", title, cwd).to_lowercase(),
        title: Some(title),
        size_bytes: metadata.len() as i64,
        file_mtime,
    }))
}

fn file_mtime_secs(path: &PathBuf) -> i64 {
    file_mtime(path).unwrap_or(0)
}

fn collect_jsonl_files(dir: &PathBuf) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_symlink() {
                continue;
            }
            if path.is_dir() {
                files.extend(collect_jsonl_files(&path));
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                files.push(path);
            }
        }
    }
    files
}

/// Scan the last 30 lines of a JSONL file for an assistant message with `ccs_proxy` marker.
fn detect_mode(lines: &[&str]) -> String {
    for line in lines.iter().rev().take(30) {
        let parsed: JsonlLine = match serde_json::from_str(line) {
            Ok(l) => l,
            Err(_) => continue,
        };
        if parsed.msg_type.as_deref() == Some("assistant") {
            if let Some(ref msg) = parsed.message {
                // Skip synthetic messages (Claude Code internal, e.g. "No response requested")
                if msg.model.as_deref() == Some("<synthetic>") {
                    continue;
                }
                if msg.ccs_proxy == Some(true) {
                    return "proxy".to_string();
                }
            }
            // First real assistant message found without proxy marker → local
            return "local".to_string();
        }
    }
    "local".to_string()
}

fn parse_session_file(path: &PathBuf, projects_dir: &Path, project_paths: &HashMap<String, PathBuf>) -> Result<Option<SessionRecord>, anyhow::Error> {
    let meta = std::fs::metadata(path)?;
    let size_bytes = meta.len() as i64;
    let file_mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| {
            let secs = d.as_secs() as i64;
            ts_to_iso(secs * 1000)
        })
        .unwrap_or_default();
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();

    if lines.is_empty() {
        return Ok(None);
    }

    let head_count = 50.min(lines.len());
    let tail_count = 30.min(lines.len());
    let message_count = lines.iter().filter(|l| !l.trim().is_empty()).count() as i64;

    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut custom_title: Option<String> = None;
    let mut ai_title: Option<String> = None;
    let mut last_prompt: Option<String> = None;

    for (i, line) in lines.iter().enumerate() {
        let in_range = i < head_count || i >= lines.len().saturating_sub(tail_count);
        if !in_range {
            if let Some(title) = parse_title_only(line) {
                match title {
                    TitleField::Custom(t) => {
                        custom_title = Some(t);
                    }
                    TitleField::Ai(t) => {
                        ai_title = Some(t);
                    }
                    TitleField::LastPrompt(t) => {
                        last_prompt = Some(truncate_title(&t));
                    }
                }
            }
            continue;
        }
        let parsed: JsonlLine = match serde_json::from_str(line) {
            Ok(l) => l,
            Err(_) => continue,
        };
        if let Some(ref sid) = parsed.session_id {
            if session_id.is_none() {
                session_id = Some(sid.clone());
            }
        }
        if let Some(ref c) = parsed.cwd {
            if cwd.is_none() {
                cwd = Some(c.clone());
            }
        }
        if let Some(ref ts) = parsed.timestamp {
            if created_at.is_none() {
                created_at = parse_timestamp(ts);
            }
        }
        if let Some(ref ct) = parsed.custom_title {
            if !ct.is_empty() {
                custom_title = Some(ct.clone());
            }
        }
        if let Some(ref at) = parsed.ai_title {
            if !at.is_empty() {
                ai_title = Some(at.clone());
            }
        }
        if let Some(ref lp) = parsed.last_prompt {
            if !lp.is_empty() {
                last_prompt = Some(truncate_title(lp));
            }
        }
    }

    let mut fallback_title: Option<String> = None;
    if custom_title.is_none() && ai_title.is_none() && last_prompt.is_none() {
        for line in lines.iter().rev().take(tail_count) {
            let parsed: JsonlLine = match serde_json::from_str(line) {
                Ok(l) => l,
                Err(_) => continue,
            };
            if parsed.msg_type.as_deref() == Some("user") {
                if let Some(ref msg) = parsed.message {
                    if let Some(ref content) = msg.content {
                        if let Some(text) = content.as_str() {
                            let t = text.trim();
                            if t.is_empty() {
                                continue;
                            }
                            if t.starts_with('<') {
                                if let Some(cmd) = extract_command(t) {
                                    fallback_title = Some(cmd);
                                    break;
                                }
                                continue;
                            }
                            fallback_title = Some(truncate_title(t));
                            break;
                        }
                    }
                }
            }
        }
    }

    let session_id = session_id.or_else(|| path.file_stem().and_then(|n| n.to_str()).map(|s| s.to_string())).unwrap_or_default();

    if session_id.is_empty() {
        return Ok(None);
    }

    let cwd = project_path_for_claude_session(path, projects_dir, project_paths)
        .map(|project_path| project_path.display().to_string())
        .or(cwd)
        .unwrap_or_default();
    let start_time = created_at.map(ts_to_iso).unwrap_or_default();
    let project_name = std::path::Path::new(&cwd).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let title = custom_title.or(ai_title).or(last_prompt).or(fallback_title).unwrap_or(project_name);

    // Detect proxy mode: scan last 30 lines for assistant message with ccs_proxy marker
    let mode = detect_mode(&lines);

    let search_text = format!("{} {}", title, cwd).to_lowercase();
    Ok(Some(SessionRecord {
        id: session_id,
        project_path: cwd,
        profile_id: None,
        mode,
        start_time,
        end_time: None,
        prompt_tokens: 0,
        completion_tokens: 0,
        message_count,
        search_text,
        title: Some(title),
        size_bytes,
        file_mtime,
    }))
}

// ── Usage Scan (background) ──────────────────────────────────────

#[derive(Debug, Deserialize)]
struct UsageLine {
    #[serde(rename = "type")]
    msg_type: Option<String>,
    message: Option<UsageMessage>,
    timestamp: Option<String>,
    #[allow(dead_code)]
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageMessage {
    id: Option<String>,
    #[allow(dead_code)]
    role: Option<String>,
    model: Option<String>,
    usage: Option<UsageData>,
    /// Actual upstream model name injected by CCSwitch proxy
    #[serde(default)]
    ccs_model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageData {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    #[allow(dead_code)]
    cache_read_input_tokens: Option<i64>,
    #[allow(dead_code)]
    cache_creation_input_tokens: Option<i64>,
}

/// Background-thread function: collect changed files, parse them, send batches via channel.
pub fn parse_files_in_background(app_type: String, ctx: ScanContext, batch_size: usize, tx: std::sync::mpsc::Sender<ScanEvent>) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let projects_dir = if app_type == "codex" {
        PathBuf::from(&home).join(".codex/sessions")
    } else {
        PathBuf::from(&home).join(".claude/projects")
    };
    let mut changed_files: Vec<(PathBuf, String)> = Vec::new();
    if projects_dir.exists() {
        collect_changed_files(&projects_dir, &mut changed_files, &ctx.file_index);
    }

    let total = changed_files.len();
    if total == 0 {
        let _ = tx.send(ScanEvent::Done {});
        return;
    }

    let mut total_records = 0usize;
    let mut last_report = 0usize;

    for (idx, (path, fallback_sid)) in changed_files.iter().enumerate() {
        let (sid, records) = if app_type == "codex" {
            parse_codex_usage_file(path, fallback_sid)
        } else {
            (fallback_sid.clone(), parse_claude_usage_file(path, fallback_sid))
        };
        let n = records.len();
        total_records += n;

        let _ = tx.send(ScanEvent::Batch {
            app_type: app_type.clone(),
            sid: sid.clone(),
            file_path: path.clone(),
            records,
        });

        let files_done = idx + 1;
        if files_done - last_report >= batch_size || files_done == total {
            let _ = tx.send(ScanEvent::Progress {
                files_done,
                files_total: total,
                records: total_records,
            });
            last_report = files_done;
        }
    }

    let _ = tx.send(ScanEvent::Done {});
}

fn parse_claude_usage_file(path: &PathBuf, fallback_sid: &str) -> Vec<UsageRecord> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let parsed: UsageLine = serde_json::from_str(line).ok()?;
            if parsed.msg_type.as_deref() != Some("assistant") {
                return None;
            }
            let msg = parsed.message.as_ref()?;
            let usage = msg.usage.as_ref()?;
            let timestamp = parsed.timestamp.as_deref().unwrap_or("");
            let msg_id = msg.id.clone().unwrap_or_else(|| format!("claude:{}:{}:{}", fallback_sid, timestamp, index));
            // Prefer ccs_model (actual upstream model from proxy) over message.model
            let model = msg.ccs_model.as_deref().or(msg.model.as_deref()).unwrap_or("unknown").replace("[1m]", "");
            if model == "<synthetic>" {
                return None;
            }
            let ts = timestamp;
            let date = normalize_usage_timestamp(ts);
            let input = usage.input_tokens.unwrap_or(0).clamp(0, MAX_USAGE_TOKENS);
            let output = usage.output_tokens.unwrap_or(0).clamp(0, MAX_USAGE_TOKENS);
            let cr = usage.cache_read_input_tokens.unwrap_or(0).clamp(0, MAX_USAGE_TOKENS);
            let cc = usage.cache_creation_input_tokens.unwrap_or(0).clamp(0, MAX_USAGE_TOKENS);
            if input == 0 && output == 0 && cr == 0 && cc == 0 {
                return None;
            }
            Some(UsageRecord {
                msg_id,
                model: model.to_string(),
                date,
                input,
                output,
                cr,
                cc,
            })
        })
        .collect()
}

fn parse_codex_usage_file(path: &PathBuf, fallback_sid: &str) -> (String, Vec<UsageRecord>) {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return (fallback_sid.to_string(), vec![]),
    };
    let mut sid = fallback_sid.to_string();
    let mut session_meta_seen = false;
    let mut model = "unknown".to_string();
    let mut records = Vec::new();
    for (index, raw) in content.lines().enumerate() {
        let line: CodexLine = match serde_json::from_str(raw) {
            Ok(line) => line,
            Err(_) => continue,
        };
        match line.line_type.as_str() {
            "session_meta" => {
                if session_meta_seen {
                    continue;
                }
                if let Some(id) = line
                    .payload
                    .get("id")
                    .or_else(|| line.payload.get("session_id"))
                    .and_then(serde_json::Value::as_str)
                    .filter(|id| !id.is_empty())
                {
                    sid = id.to_string();
                    session_meta_seen = true;
                }
            }
            "turn_context" => {
                if let Some(value) = line.payload.get("model").and_then(serde_json::Value::as_str) {
                    model = value.to_string();
                }
            }
            "event_msg" if line.payload.get("type").and_then(serde_json::Value::as_str) == Some("token_count") => {
                let Some(usage) = line.payload.get("info").and_then(|info| info.get("last_token_usage")) else {
                    continue;
                };
                let timestamp = line.timestamp.as_deref().unwrap_or("");
                let msg_id = format!("codex:{}:{}:{}", sid, timestamp, index);
                let input = usage.get("input_tokens").and_then(serde_json::Value::as_i64).unwrap_or(0).clamp(0, MAX_USAGE_TOKENS);
                let cached = usage.get("cached_input_tokens").and_then(serde_json::Value::as_i64).unwrap_or(0).clamp(0, input);
                let output = usage.get("output_tokens").and_then(serde_json::Value::as_i64).unwrap_or(0).clamp(0, MAX_USAGE_TOKENS);
                records.push(UsageRecord {
                    msg_id,
                    model: model.clone(),
                    date: normalize_usage_timestamp(timestamp),
                    input: input.saturating_sub(cached),
                    output,
                    cr: cached,
                    cc: 0,
                });
            }
            _ => {}
        }
    }
    (sid, records)
}

fn normalize_usage_timestamp(timestamp: &str) -> String {
    let date = timestamp
        .get(..10)
        .filter(|value| value.is_ascii() && value.as_bytes().get(4) == Some(&b'-') && value.as_bytes().get(7) == Some(&b'-'));
    let time = timestamp
        .get(11..19)
        .filter(|value| value.is_ascii() && value.as_bytes().get(2) == Some(&b':') && value.as_bytes().get(5) == Some(&b':'));
    match (date, time) {
        (Some(date), Some(time)) => format!("{} {}", date, time),
        (Some(date), None) => format!("{} 00:00:00", date),
        _ => "today".to_string(),
    }
}

fn normalize_session_timestamp(timestamp: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|dt| dt.with_timezone(&chrono::Utc).format("%Y-%m-%d %H:%M:%S").to_string())
        .ok()
}

// ── Helpers ──────────────────────────────────────────────────────

pub fn collect_changed_files(dir: &PathBuf, out: &mut Vec<(PathBuf, String)>, file_index: &HashMap<String, i64>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_symlink() {
                continue;
            }
            if path.is_dir() {
                collect_changed_files(&path, out, file_index);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                let sid = path.file_stem().and_then(|n| n.to_str()).unwrap_or("").to_string();
                if !sid.is_empty() {
                    let mtime = file_mtime(&path).unwrap_or(0);
                    let file_path_str = path.to_string_lossy().to_string();
                    let changed = file_index.get(&file_path_str).map_or(true, |&old| old != mtime);
                    if changed {
                        out.push((path, sid));
                    }
                }
            }
        }
    }
}

/// Read file mtime as unix timestamp (seconds) — public for db/usage.rs
pub fn file_mtime(path: &PathBuf) -> Option<i64> {
    let meta = std::fs::metadata(path).ok()?;
    let dur = meta.modified().ok()?;
    let secs = dur.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(secs.as_nanos()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_project_directory_overrides_stale_or_missing_jsonl_cwd() {
        let root = tempfile::tempdir().unwrap();
        let projects_dir = root.path().join("claude-projects");
        let current_project = root.path().join("current-project");
        let encoded_project = "encoded-current-project";
        let session_dir = projects_dir.join(encoded_project);
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::create_dir(&current_project).unwrap();
        let project_paths = HashMap::from([(encoded_project.to_string(), current_project.clone())]);

        for (session_id, cwd_field) in [("stale-cwd", r#", "cwd":"/old/project/path""#), ("missing-cwd", "")] {
            let path = session_dir.join(format!("{session_id}.jsonl"));
            std::fs::write(&path, format!(r#"{{"type":"ai-title","sessionId":"{session_id}","aiTitle":"Session"{cwd_field}}}"#)).unwrap();

            let session = parse_session_file(&path, &projects_dir, &project_paths).unwrap().unwrap();

            assert_eq!(session.project_path, current_project.display().to_string());
        }
    }

    #[test]
    fn loads_unique_existing_claude_project_paths() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let config_path = root.path().join(".claude.json");
        std::fs::write(
            &config_path,
            serde_json::json!({
                "projects": {
                    project.display().to_string(): {},
                    root.path().join("missing").display().to_string(): {}
                }
            })
            .to_string(),
        )
        .unwrap();

        let projects = load_claude_project_paths(&config_path);

        assert_eq!(projects.len(), 1);
        assert_eq!(projects.get(&claude_project_directory_name(&project.to_string_lossy())), Some(&project));
    }

    #[test]
    fn claude_import_revision_invalidates_only_session_scan_once() {
        let root = tempfile::tempdir().unwrap();
        let projects_dir = root.path().join("projects");
        let claude_path = projects_dir.join("session.jsonl");
        let unrelated_path = root.path().join("codex.jsonl");
        std::fs::create_dir(&projects_dir).unwrap();
        let db = Db::open(&root.path().join("ccswitch.db")).unwrap();
        for (path, scan_type) in [(&claude_path, "session"), (&claude_path, "usage"), (&unrelated_path, "session")] {
            db.conn()
                .execute(
                    "INSERT INTO session_log_sync (file_path, file_mtime, scan_type) VALUES (?1, 1, ?2)",
                    rusqlite::params![path.to_string_lossy(), scan_type],
                )
                .unwrap();
        }

        assert!(prepare_claude_import_revision(&db, &projects_dir).unwrap());
        let scan_rows = {
            let mut statement = db
                .conn()
                .prepare("SELECT file_path, scan_type FROM session_log_sync ORDER BY file_path, scan_type")
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(
            scan_rows,
            vec![
                (unrelated_path.to_string_lossy().to_string(), "session".into()),
                (claude_path.to_string_lossy().to_string(), "usage".into()),
            ]
        );

        db.conn()
            .execute(
                "INSERT INTO session_log_sync (file_path, file_mtime, scan_type) VALUES (?1, 1, 'session')",
                [claude_path.to_string_lossy().as_ref()],
            )
            .unwrap();
        assert!(!prepare_claude_import_revision(&db, &projects_dir).unwrap());
        let session_scan_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM session_log_sync WHERE file_path = ?1 AND scan_type = 'session'",
                [claude_path.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(session_scan_count, 1);
    }

    #[test]
    fn parses_codex_session_and_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-test-session-1.jsonl");
        std::fs::write(&path, concat!(
            r#"{"timestamp":"2026-07-26T12:00:00Z","type":"session_meta","payload":{"id":"session-1","cwd":"/tmp/project","model_provider":"provider-1","timestamp":"2026-07-26T12:00:00Z"}}"#, "\n",
            r#"{"timestamp":"2026-07-26T12:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Implement the feature"}}"#, "\n",
            r#"{"timestamp":"2026-07-26T12:00:02Z","type":"turn_context","payload":{"model":"gpt-test"}}"#, "\n",
            r#"{"timestamp":"2026-07-26T12:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":40,"output_tokens":20,"reasoning_output_tokens":5,"total_tokens":120}}}}"#, "\n",
            r#"{"timestamp":"2026-07-26T12:00:04Z","type":"event_msg","payload":{"type":"agent_message","message":"Done"}}"#, "\n",
        )).unwrap();

        let session = parse_codex_session_file(&path).unwrap().unwrap();
        assert_eq!(session.id, "session-1");
        assert_eq!(session.profile_id.as_deref(), Some("provider-1"));
        assert_eq!(session.mode, "direct");
        assert_eq!(session.title.as_deref(), Some("Implement the feature"));
        assert_eq!(session.message_count, 2);

        let (sid, records) = parse_codex_usage_file(&path, "fallback");
        assert_eq!(sid, "session-1");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model, "gpt-test");
        assert_eq!(records[0].input, 60);
        assert_eq!(records[0].cr, 40);
        assert_eq!(records[0].output, 20);

        let index_path = dir.path().join("session_index.jsonl");
        std::fs::write(&index_path, r#"{"id":"session-1","thread_name":"Renamed session","updated_at":"2026-07-26T13:00:00Z"}"#).unwrap();
        let index = load_codex_session_index(&index_path);
        assert_eq!(index.get("session-1").map(String::as_str), Some("Renamed session"));
        let db = Db::open(&dir.path().join("ccswitch.db")).unwrap();
        db.insert_session(&session, "codex").unwrap();
        apply_codex_session_index(&db, &index).unwrap();
        let stored = db.query_sessions("codex", None, None, 10).unwrap();
        assert_eq!(stored[0].title.as_deref(), Some("Renamed session"));
    }

    #[test]
    fn codex_fork_uses_first_session_meta_as_canonical_id() {
        let dir = tempfile::tempdir().unwrap();
        let parent_path = dir.path().join("rollout-parent-session.jsonl");
        let fork_path = dir.path().join("rollout-fork-session.jsonl");
        std::fs::write(
            &parent_path,
            concat!(
                r#"{"timestamp":"2026-07-23T08:00:00Z","type":"session_meta","payload":{"id":"parent-session","cwd":"/tmp/parent","model_provider":"parent-provider"}}"#,
                "\n",
                r#"{"timestamp":"2026-07-23T08:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Parent prompt"}}"#,
                "\n"
            ),
        )
        .unwrap();
        std::fs::write(
            &fork_path,
            concat!(
                r#"{"timestamp":"2026-07-27T08:00:00Z","type":"session_meta","payload":{"id":"fork-session","cwd":"/tmp/fork","model_provider":"fork-provider","forked_from_id":"parent-session"}}"#,
                "\n",
                r#"{"timestamp":"2026-07-23T08:00:00Z","type":"session_meta","payload":{"id":"parent-session","cwd":"/tmp/parent","model_provider":"parent-provider"}}"#,
                "\n",
                r#"{"timestamp":"2026-07-27T08:00:01Z","type":"turn_context","payload":{"model":"gpt-fork"}}"#,
                "\n",
                r#"{"timestamp":"2026-07-27T08:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":80,"cached_input_tokens":20,"output_tokens":10}}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let parent = parse_codex_session_file(&parent_path).unwrap().unwrap();
        let fork = parse_codex_session_file(&fork_path).unwrap().unwrap();
        assert_eq!(fork.id, "fork-session");
        assert_eq!(fork.project_path, "/tmp/fork");
        assert_eq!(fork.profile_id.as_deref(), Some("fork-provider"));

        let (sid, records) = parse_codex_usage_file(&fork_path, "fallback");
        assert_eq!(sid, "fork-session");
        assert_eq!(records.len(), 1);
        assert!(records[0].msg_id.starts_with("codex:fork-session:"));

        let db = Db::open(&dir.path().join("ccswitch.db")).unwrap();
        db.insert_session(&parent, "codex").unwrap();
        db.insert_session(&fork, "codex").unwrap();
        let stored = db.query_sessions("codex", None, None, 10).unwrap();
        assert_eq!(stored.len(), 2);
        assert!(stored.iter().any(|session| session.id == "parent-session"));
        assert!(stored.iter().any(|session| session.id == "fork-session"));
    }

    #[test]
    fn codex_import_revision_only_invalidates_codex_import_state_once() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("codex-sessions");
        let codex_path = sessions_dir.join("rollout-session.jsonl");
        let unrelated_path = dir.path().join("claude-session.jsonl");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        let db = Db::open(&dir.path().join("ccswitch.db")).unwrap();
        for (app_type, source, message_id) in [
            ("codex", "import", "codex-import"),
            ("codex", "manual", "codex-manual"),
            ("claude", "import", "claude-import"),
        ] {
            db.conn()
                .execute(
                    "INSERT INTO usage_logs (app_type, model, message_id, data_source) VALUES (?1, 'model', ?2, ?3)",
                    rusqlite::params![app_type, message_id, source],
                )
                .unwrap();
        }
        for (path, scan_type) in [(&codex_path, "session"), (&codex_path, "usage"), (&unrelated_path, "session")] {
            db.conn()
                .execute(
                    "INSERT INTO session_log_sync (file_path, file_mtime, scan_type) VALUES (?1, 1, ?2)",
                    rusqlite::params![path.to_string_lossy(), scan_type],
                )
                .unwrap();
        }

        prepare_codex_import_revision(&db, &sessions_dir).unwrap();

        let usage_ids = {
            let mut statement = db.conn().prepare("SELECT message_id FROM usage_logs ORDER BY message_id").unwrap();
            statement.query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert_eq!(usage_ids, vec!["claude-import", "codex-manual"]);
        let sync_paths = {
            let mut statement = db.conn().prepare("SELECT file_path FROM session_log_sync").unwrap();
            statement.query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert_eq!(sync_paths, vec![unrelated_path.to_string_lossy()]);
        assert_eq!(db.get_setting(CODEX_IMPORT_REVISION_KEY).as_deref(), Some(CODEX_IMPORT_REVISION));

        db.conn()
            .execute(
                "INSERT INTO usage_logs (app_type, model, message_id, data_source) VALUES ('codex', 'model', 'new-import', 'import')",
                [],
            )
            .unwrap();
        prepare_codex_import_revision(&db, &sessions_dir).unwrap();
        let new_import_count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM usage_logs WHERE message_id = 'new-import'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(new_import_count, 1);
    }

    #[test]
    fn anonymous_claude_usage_ids_are_stable_and_refresh_replaces_old_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-1.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"assistant","timestamp":"2026-07-27T10:00:00Z","message":{"model":"claude-sonnet","usage":{"input_tokens":10,"output_tokens":2}}}"#,
                "\n",
                r#"{"type":"assistant","timestamp":"2026-07-27T10:00:01Z","message":{"model":"claude-sonnet","usage":{"input_tokens":-5,"output_tokens":0}}}"#,
                "\n",
                r#"{"type":"assistant","timestamp":"时间戳","message":{"model":"claude-sonnet","usage":{"input_tokens":9223372036854775807,"output_tokens":1}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let records = parse_claude_usage_file(&path, "session-1");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].msg_id, "claude:session-1:2026-07-27T10:00:00Z:0");
        assert_eq!(records[1].date, "today");
        assert_eq!(records[1].input, MAX_USAGE_TOKENS);

        let reparsed = parse_claude_usage_file(&path, "session-1");
        assert_eq!(
            records.iter().map(|record| &record.msg_id).collect::<Vec<_>>(),
            reparsed.iter().map(|record| &record.msg_id).collect::<Vec<_>>()
        );

        let db = Db::open(&dir.path().join("ccswitch.db")).unwrap();
        db.insert_usage_batch("claude", "session-1", &path, &records).unwrap();
        let replacement = vec![UsageRecord {
            msg_id: records[0].msg_id.clone(),
            model: records[0].model.clone(),
            date: records[0].date.clone(),
            input: 5,
            output: 1,
            cr: 0,
            cc: 0,
        }];
        db.insert_usage_batch("claude", "session-1", &path, &replacement).unwrap();
        let summary = db.query_usage("claude", "all").unwrap();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].total_prompt, 5);
        assert_eq!(summary[0].request_count, 1);
    }

    #[test]
    fn removed_session_files_clear_session_usage_and_sync_state() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        std::fs::create_dir_all(&root).unwrap();
        let session_path = root.join("session-1.jsonl");
        std::fs::write(&session_path, "{}\n").unwrap();

        let db = Db::open(&dir.path().join("ccswitch.db")).unwrap();
        db.insert_session(
            &SessionRecord {
                id: "session-1".into(),
                project_path: "/tmp/project".into(),
                profile_id: None,
                mode: "local".into(),
                start_time: "2026-07-27 10:00:00".into(),
                end_time: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                message_count: 1,
                title: Some("Session".into()),
                size_bytes: 3,
                file_mtime: "2026-07-27 10:00:00".into(),
                search_text: String::new(),
            },
            "claude",
        )
        .unwrap();
        db.conn()
            .execute(
                "INSERT INTO usage_logs (app_type, model, session_id, message_id, prompt_tokens) VALUES ('claude', 'model', 'session-1', 'msg-1', 10)",
                [],
            )
            .unwrap();
        let path_text = session_path.to_string_lossy().to_string();
        for scan_type in ["session", "usage"] {
            db.conn()
                .execute(
                    "INSERT INTO session_log_sync (file_path, file_mtime, scan_type) VALUES (?1, 1, ?2)",
                    rusqlite::params![path_text, scan_type],
                )
                .unwrap();
        }
        std::fs::remove_file(&session_path).unwrap();

        let file_index = HashMap::from([(path_text, 1)]);
        assert_eq!(cleanup_removed_session_files(&db, &root, "claude", &file_index).unwrap(), 1);
        assert!(db.query_sessions("claude", None, None, 10).unwrap().is_empty());
        let usage_count: i64 = db.conn().query_row("SELECT COUNT(*) FROM usage_logs", [], |row| row.get(0)).unwrap();
        let sync_count: i64 = db.conn().query_row("SELECT COUNT(*) FROM session_log_sync", [], |row| row.get(0)).unwrap();
        assert_eq!(usage_count, 0);
        assert_eq!(sync_count, 0);
    }
}
