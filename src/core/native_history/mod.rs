//! Native History ingestion from Claude Code and Codex JSONL files.
//!
//! The public interface owns discovery, revision, cleanup, persistence, and
//! progress semantics. Claude and Codex file formats remain internal adapters.

use std::{
    collections::HashMap,
    io::BufRead,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
};

const MAX_USAGE_TOKENS: i64 = 1_000_000_000_000;
const CLAUDE_IMPORT_REVISION_KEY: &str = "claude_import_revision";
const CLAUDE_IMPORT_REVISION: &str = "1";
const CODEX_IMPORT_REVISION_KEY: &str = "codex_import_revision";
const CODEX_IMPORT_REVISION: &str = "4";
const CLAUDE_SESSION_REVISION: &str = "1";
const CLAUDE_USAGE_REVISION: &str = "1";
const CODEX_SESSION_REVISION: &str = "4";
const CODEX_USAGE_REVISION: &str = "4";
const USAGE_PROGRESS_BATCH_SIZE: usize = 10;

use serde::Deserialize;

use crate::agent::AgentKind;
use crate::db::connection::Db;
use crate::db::sessions::SessionRecord;

#[derive(Debug, Clone)]
pub struct IngestionProgress {
    pub files_done: usize,
    pub files_total: usize,
    pub records: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestionReport {
    pub processed: usize,
    pub changed: usize,
    pub removed: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug)]
pub enum UsageScanUpdate {
    Running(IngestionProgress),
    Complete(IngestionReport),
    Failed(String),
}

#[derive(Debug, Clone)]
struct UsageRecord {
    msg_id: String,
    model: String,
    date: String,
    input: i64,
    output: i64,
    cr: i64,
    cc: i64,
}

#[derive(Debug, Clone)]
struct NativeRoots {
    claude_projects: PathBuf,
    claude_config: PathBuf,
    codex_sessions: PathBuf,
    codex_session_index: PathBuf,
}

impl NativeRoots {
    fn system() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let home = PathBuf::from(home);
        Self {
            claude_projects: home.join(".claude/projects"),
            claude_config: home.join(".claude.json"),
            codex_sessions: home.join(".codex/sessions"),
            codex_session_index: home.join(".codex/session_index.jsonl"),
        }
    }
}

pub struct NativeHistoryIngestion<'db> {
    db: &'db Db,
    roots: NativeRoots,
}

trait NativeHistoryAdapter: Send {
    fn agent(&self) -> AgentKind;
    fn session_root(&self) -> &Path;
    fn accepts_session_file(&self, path: &Path) -> bool;
    fn accepts_usage_file(&self, _path: &Path) -> bool {
        true
    }
    fn parse_session(&self, path: &Path) -> anyhow::Result<Option<SessionRecord>>;
    fn finish_session_scan(&self, db: &Db) -> anyhow::Result<usize>;
    fn parse_usage(&self, path: &Path, fallback_session_id: &str) -> anyhow::Result<(String, Vec<UsageRecord>)>;
}

struct ClaudeHistoryAdapter {
    projects_dir: PathBuf,
    project_paths: HashMap<String, PathBuf>,
}

impl ClaudeHistoryAdapter {
    fn new(roots: &NativeRoots) -> Self {
        Self {
            projects_dir: roots.claude_projects.clone(),
            project_paths: load_claude_project_paths(&roots.claude_config),
        }
    }
}

impl NativeHistoryAdapter for ClaudeHistoryAdapter {
    fn agent(&self) -> AgentKind {
        AgentKind::Claude
    }

    fn session_root(&self) -> &Path {
        &self.projects_dir
    }

    fn accepts_session_file(&self, path: &Path) -> bool {
        !path.file_stem().and_then(|name| name.to_str()).is_some_and(|id| id.starts_with("agent-"))
    }

    fn parse_session(&self, path: &Path) -> anyhow::Result<Option<SessionRecord>> {
        parse_session_file(path, &self.projects_dir, &self.project_paths)
    }

    fn finish_session_scan(&self, _db: &Db) -> anyhow::Result<usize> {
        Ok(0)
    }

    fn parse_usage(&self, path: &Path, fallback_session_id: &str) -> anyhow::Result<(String, Vec<UsageRecord>)> {
        Ok((fallback_session_id.to_string(), parse_claude_usage_file(path, fallback_session_id)?))
    }
}

struct CodexHistoryAdapter {
    sessions_dir: PathBuf,
    session_index: HashMap<String, String>,
}

impl CodexHistoryAdapter {
    fn new(roots: &NativeRoots) -> Self {
        Self {
            sessions_dir: roots.codex_sessions.clone(),
            session_index: load_codex_session_index(&roots.codex_session_index),
        }
    }
}

impl NativeHistoryAdapter for CodexHistoryAdapter {
    fn agent(&self) -> AgentKind {
        AgentKind::Codex
    }

    fn session_root(&self) -> &Path {
        &self.sessions_dir
    }

    fn accepts_session_file(&self, _path: &Path) -> bool {
        true
    }

    fn parse_session(&self, path: &Path) -> anyhow::Result<Option<SessionRecord>> {
        parse_codex_session_file(path)
    }

    fn finish_session_scan(&self, db: &Db) -> anyhow::Result<usize> {
        apply_codex_session_index(db, &self.session_index).map_err(Into::into)
    }

    fn parse_usage(&self, path: &Path, fallback_session_id: &str) -> anyhow::Result<(String, Vec<UsageRecord>)> {
        parse_codex_usage_file(path, fallback_session_id)
    }
}

fn adapter(agent: AgentKind, roots: &NativeRoots) -> Box<dyn NativeHistoryAdapter> {
    match agent {
        AgentKind::Claude => Box::new(ClaudeHistoryAdapter::new(roots)),
        AgentKind::Codex => Box::new(CodexHistoryAdapter::new(roots)),
    }
}

#[derive(Clone, Copy)]
enum NativeDataset {
    Sessions,
    Usage,
}

impl NativeDataset {
    fn scan_type(self) -> &'static str {
        match self {
            Self::Sessions => "session",
            Self::Usage => "usage",
        }
    }

    fn revision_name(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::Usage => "usage",
        }
    }
}

impl<'db> NativeHistoryIngestion<'db> {
    pub fn new(db: &'db Db) -> Self {
        Self { db, roots: NativeRoots::system() }
    }

    #[cfg(test)]
    fn with_roots(db: &'db Db, roots: NativeRoots) -> Self {
        Self { db, roots }
    }

    pub fn refresh_sessions(&self, agent: AgentKind, mut on_progress: impl FnMut(IngestionProgress)) -> anyhow::Result<IngestionReport> {
        let adapter = adapter(agent, &self.roots);
        ensure_revision(self.db, adapter.as_ref(), NativeDataset::Sessions)?;
        let file_index = load_sync_index(self.db, NativeDataset::Sessions)?;
        let mut report = IngestionReport {
            removed: cleanup_removed_session_files(self.db, adapter.session_root(), agent_key(agent), &file_index)?,
            ..IngestionReport::default()
        };
        let files = collect_jsonl_files(adapter.session_root())
            .into_iter()
            .filter(|path| adapter.accepts_session_file(path))
            .collect::<Vec<_>>();
        let files_total = files.len();

        for (index, path) in files.iter().enumerate() {
            report.processed += 1;
            let path_text = path.to_string_lossy().to_string();
            let mtime = file_mtime(path).unwrap_or(0);
            if file_index.get(&path_text) == Some(&mtime) {
                report.skipped += 1;
            } else {
                match adapter.parse_session(path) {
                    Ok(Some(record)) => {
                        persist_session_file(self.db, agent, &record, path, mtime)?;
                        report.changed += 1;
                    }
                    Ok(None) => {
                        report.failed += 1;
                        tracing::warn!(path = %path.display(), "Native History session file did not contain a session record");
                    }
                    Err(error) => {
                        report.failed += 1;
                        tracing::warn!(path = %path.display(), %error, "Failed to parse Native History session file");
                    }
                }
            }
            on_progress(IngestionProgress {
                files_done: index + 1,
                files_total,
                records: report.changed,
            });
        }
        adapter.finish_session_scan(self.db)?;
        if files_total == 0 {
            on_progress(IngestionProgress {
                files_done: 0,
                files_total: 0,
                records: 0,
            });
        }
        Ok(report)
    }

    pub fn refresh_titles(&self) -> anyhow::Result<IngestionReport> {
        let mut report = self.refresh_sessions(AgentKind::Claude, |_| {})?;
        let codex = adapter(AgentKind::Codex, &self.roots);
        report.changed += codex.finish_session_scan(self.db)?;
        Ok(report)
    }

    pub fn start_usage_scan(&self, agent: AgentKind) -> anyhow::Result<UsageScan> {
        let adapter = adapter(agent, &self.roots);
        ensure_revision(self.db, adapter.as_ref(), NativeDataset::Usage)?;
        self.db.conn().execute("DELETE FROM usage_logs WHERE model = '<synthetic>'", [])?;
        let file_index = load_sync_index(self.db, NativeDataset::Usage)?;
        let report = IngestionReport {
            removed: cleanup_removed_session_files(self.db, adapter.session_root(), agent_key(agent), &file_index)?,
            ..IngestionReport::default()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        let handle = std::thread::spawn(move || usage_worker(adapter, file_index, worker_cancel, sender));
        Ok(UsageScan {
            agent,
            receiver,
            handle: Some(handle),
            cancel,
            report,
            terminal: false,
        })
    }
}

enum UsageWorkerEvent {
    Started { skipped: usize },
    File { path: PathBuf, session_id: String, records: Vec<UsageRecord> },
    FileFailed { path: PathBuf, message: String },
    Progress(IngestionProgress),
    Done,
}

pub struct UsageScan {
    agent: AgentKind,
    receiver: mpsc::Receiver<UsageWorkerEvent>,
    handle: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    report: IngestionReport,
    terminal: bool,
}

impl UsageScan {
    pub fn poll(&mut self, db: &Db) -> Option<UsageScanUpdate> {
        if self.terminal {
            return None;
        }
        let mut latest_progress = None;
        loop {
            match self.receiver.try_recv() {
                Ok(UsageWorkerEvent::Started { skipped }) => self.report.skipped = skipped,
                Ok(UsageWorkerEvent::File { path, session_id, records }) => {
                    self.report.processed += 1;
                    if let Err(error) = persist_usage_file(db, self.agent, &session_id, &path, &records) {
                        return Some(self.fail(error.to_string()));
                    }
                    self.report.changed += 1;
                }
                Ok(UsageWorkerEvent::FileFailed { path, message }) => {
                    self.report.processed += 1;
                    self.report.failed += 1;
                    tracing::warn!(path = %path.display(), error = %message, "Failed to parse Native History usage file");
                }
                Ok(UsageWorkerEvent::Progress(progress)) => latest_progress = Some(progress),
                Ok(UsageWorkerEvent::Done) => {
                    self.join_worker();
                    self.terminal = true;
                    return Some(UsageScanUpdate::Complete(self.report.clone()));
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Some(self.fail("Native History usage worker stopped unexpectedly".into()));
                }
            }
        }
        latest_progress.map(UsageScanUpdate::Running)
    }

    pub fn shutdown(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.join_worker();
        self.terminal = true;
    }

    fn fail(&mut self, message: String) -> UsageScanUpdate {
        self.cancel.store(true, Ordering::Relaxed);
        self.join_worker();
        self.terminal = true;
        UsageScanUpdate::Failed(message)
    }

    fn join_worker(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for UsageScan {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn usage_worker(adapter: Box<dyn NativeHistoryAdapter>, file_index: HashMap<String, i64>, cancel: Arc<AtomicBool>, sender: mpsc::Sender<UsageWorkerEvent>) {
    let files = collect_jsonl_files(adapter.session_root())
        .into_iter()
        .filter(|path| adapter.accepts_usage_file(path))
        .collect::<Vec<_>>();
    let mut changed_files = Vec::new();
    let mut skipped = 0;
    for path in files {
        let path_text = path.to_string_lossy().to_string();
        if file_index.get(&path_text) == file_mtime(&path).as_ref() {
            skipped += 1;
        } else {
            changed_files.push(path);
        }
    }
    if sender.send(UsageWorkerEvent::Started { skipped }).is_err() {
        return;
    }
    let files_total = changed_files.len();
    let mut records_total = 0;
    let mut last_report = 0;
    for (index, path) in changed_files.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let fallback_session_id = path.file_stem().and_then(|name| name.to_str()).unwrap_or("");
        match adapter.parse_usage(&path, fallback_session_id) {
            Ok((session_id, records)) => {
                records_total += records.len();
                if sender
                    .send(UsageWorkerEvent::File {
                        path: path.clone(),
                        session_id,
                        records,
                    })
                    .is_err()
                {
                    return;
                }
            }
            Err(error) => {
                if sender
                    .send(UsageWorkerEvent::FileFailed {
                        path: path.clone(),
                        message: error.to_string(),
                    })
                    .is_err()
                {
                    return;
                }
            }
        }
        let files_done = index + 1;
        if files_done - last_report >= USAGE_PROGRESS_BATCH_SIZE || files_done == files_total {
            if sender
                .send(UsageWorkerEvent::Progress(IngestionProgress {
                    files_done,
                    files_total,
                    records: records_total,
                }))
                .is_err()
            {
                return;
            }
            last_report = files_done;
        }
    }
    let _ = sender.send(UsageWorkerEvent::Done);
}

fn agent_key(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
    }
}

fn load_sync_index(db: &Db, dataset: NativeDataset) -> anyhow::Result<HashMap<String, i64>> {
    let mut statement = db.conn().prepare("SELECT file_path, file_mtime FROM session_log_sync WHERE scan_type = ?1")?;
    let rows = statement.query_map([dataset.scan_type()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn ensure_revision(db: &Db, adapter: &dyn NativeHistoryAdapter, dataset: NativeDataset) -> anyhow::Result<()> {
    let agent = adapter.agent();
    let revision = match (agent, dataset) {
        (AgentKind::Claude, NativeDataset::Sessions) => CLAUDE_SESSION_REVISION,
        (AgentKind::Claude, NativeDataset::Usage) => CLAUDE_USAGE_REVISION,
        (AgentKind::Codex, NativeDataset::Sessions) => CODEX_SESSION_REVISION,
        (AgentKind::Codex, NativeDataset::Usage) => CODEX_USAGE_REVISION,
    };
    let key = format!("native_history_revision:{}:{}", agent_key(agent), dataset.revision_name());
    if db.get_setting(&key).as_deref() == Some(revision) {
        return Ok(());
    }
    let legacy_is_current = match (agent, dataset) {
        (AgentKind::Claude, NativeDataset::Sessions) => db.get_setting(CLAUDE_IMPORT_REVISION_KEY).as_deref() == Some(CLAUDE_IMPORT_REVISION),
        (AgentKind::Claude, NativeDataset::Usage) => true,
        (AgentKind::Codex, _) => db.get_setting(CODEX_IMPORT_REVISION_KEY).as_deref() == Some(CODEX_IMPORT_REVISION),
    };
    let transaction = db.conn().unchecked_transaction()?;
    if !legacy_is_current {
        match dataset {
            NativeDataset::Sessions => {
                transaction.execute("DELETE FROM session_history WHERE app_type = ?1", [agent_key(agent)])?;
            }
            NativeDataset::Usage => {
                transaction.execute("DELETE FROM usage_logs WHERE app_type = ?1 AND data_source = 'import'", [agent_key(agent)])?;
            }
        }
        let paths = {
            let mut statement = transaction.prepare("SELECT file_path FROM session_log_sync WHERE scan_type = ?1")?;
            let paths = statement.query_map([dataset.scan_type()], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            paths
        };
        for path in paths.into_iter().filter(|path| Path::new(path).starts_with(adapter.session_root())) {
            transaction.execute(
                "DELETE FROM session_log_sync WHERE file_path = ?1 AND scan_type = ?2",
                rusqlite::params![path, dataset.scan_type()],
            )?;
        }
    }
    transaction.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)", rusqlite::params![key, revision])?;
    transaction.commit()?;
    Ok(())
}

fn persist_session_file(db: &Db, agent: AgentKind, session: &SessionRecord, path: &Path, mtime: i64) -> anyhow::Result<()> {
    let transaction = db.conn().unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO session_history (id, app_type, project_path, profile_id, parent_thread_id, mode, start_time, end_time, prompt_tokens, completion_tokens, message_count, title, size_bytes, file_mtime)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(id, app_type) DO UPDATE SET
            project_path=excluded.project_path, profile_id=excluded.profile_id,
            parent_thread_id=excluded.parent_thread_id,
            mode=excluded.mode, start_time=excluded.start_time, end_time=excluded.end_time,
            prompt_tokens=excluded.prompt_tokens, completion_tokens=excluded.completion_tokens,
            message_count=excluded.message_count, title=excluded.title,
            size_bytes=excluded.size_bytes, file_mtime=excluded.file_mtime",
        rusqlite::params![
            session.id,
            agent_key(agent),
            session.project_path,
            session.profile_id,
            session.parent_thread_id,
            session.mode,
            session.start_time,
            session.end_time,
            session.prompt_tokens,
            session.completion_tokens,
            session.message_count,
            session.title,
            session.size_bytes,
            session.file_mtime
        ],
    )?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    transaction.execute(
        "INSERT INTO session_log_sync (file_path, file_mtime, scan_type, last_synced_at)
         VALUES (?1, ?2, 'session', ?3)
         ON CONFLICT(file_path, scan_type) DO UPDATE SET
            file_mtime=excluded.file_mtime,
            last_synced_at=excluded.last_synced_at",
        rusqlite::params![path.to_string_lossy(), mtime, now],
    )?;
    transaction.commit()?;
    Ok(())
}

fn persist_usage_file(db: &Db, agent: AgentKind, session_id: &str, path: &Path, records: &[UsageRecord]) -> anyhow::Result<()> {
    let mtime = file_mtime(path).ok_or_else(|| anyhow::anyhow!("Usage file metadata is unavailable"))?;
    let transaction = db.conn().unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM usage_logs WHERE app_type = ?1 AND session_id = ?2 AND data_source = 'import'",
        rusqlite::params![agent_key(agent), session_id],
    )?;
    for record in records {
        transaction.execute(
            "INSERT OR IGNORE INTO usage_logs (app_type, model, provider_id, profile_id, session_id, message_id,
             prompt_tokens, completion_tokens, cache_read_tokens, cache_creation_tokens, total_tokens, timestamp, data_source)
             VALUES (?1, ?2, '', '', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'import')",
            rusqlite::params![
                agent_key(agent),
                record.model,
                session_id,
                record.msg_id,
                record.input,
                record.output,
                record.cr,
                record.cc,
                record.input.saturating_add(record.output).saturating_add(record.cr).saturating_add(record.cc),
                record.date
            ],
        )?;
    }
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    transaction.execute(
        "INSERT INTO session_log_sync (file_path, file_mtime, scan_type, last_synced_at)
         VALUES (?1, ?2, 'usage', ?3)
         ON CONFLICT(file_path, scan_type) DO UPDATE SET
            file_mtime=excluded.file_mtime,
            last_synced_at=excluded.last_synced_at",
        rusqlite::params![path.to_string_lossy(), mtime, now],
    )?;
    transaction.commit()?;
    Ok(())
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodexThreadOrigin {
    Primary,
    Child,
}

#[derive(Debug)]
struct CodexSessionMetadata {
    id: String,
    cwd: String,
    provider: String,
    parent_thread_id: Option<String>,
    start_time: Option<String>,
}

fn parse_codex_session_metadata_line(raw: &str) -> Option<CodexSessionMetadata> {
    let line: CodexLine = serde_json::from_str(raw).ok()?;
    if line.line_type != "session_meta" {
        return None;
    }
    let id = line
        .payload
        .get("id")
        .or_else(|| line.payload.get("session_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())?;
    Some(CodexSessionMetadata {
        id: id.to_string(),
        cwd: line.payload.get("cwd").and_then(serde_json::Value::as_str).unwrap_or("").to_string(),
        provider: line.payload.get("model_provider").and_then(serde_json::Value::as_str).unwrap_or("").to_string(),
        parent_thread_id: line
            .payload
            .get("parent_thread_id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        start_time: line.payload.get("timestamp").and_then(serde_json::Value::as_str).and_then(normalize_session_timestamp),
    })
}

fn parse_codex_session_metadata(content: &str) -> Option<CodexSessionMetadata> {
    content.lines().find_map(parse_codex_session_metadata_line)
}

pub(crate) fn codex_thread_origin(thread_id: &str) -> anyhow::Result<Option<CodexThreadOrigin>> {
    let root = NativeRoots::system().codex_sessions;
    codex_thread_origin_in(&root, thread_id)
}

fn codex_thread_origin_in(root: &Path, thread_id: &str) -> anyhow::Result<Option<CodexThreadOrigin>> {
    if thread_id.is_empty() || !thread_id.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')) {
        anyhow::bail!("Invalid Codex thread identifier");
    }
    let Some(path) = find_codex_session_file(root, thread_id, 10)? else {
        return Ok(None);
    };
    let reader = std::io::BufReader::new(std::fs::File::open(&path)?);
    let mut metadata = None;
    for line in reader.lines().take(64) {
        if let Some(found) = parse_codex_session_metadata_line(&line?) {
            metadata = Some(found);
            break;
        }
    }
    let metadata = metadata.ok_or_else(|| anyhow::anyhow!("Codex rollout has no canonical session metadata"))?;
    anyhow::ensure!(metadata.id == thread_id, "Codex rollout metadata does not match the completion thread");
    Ok(Some(if metadata.parent_thread_id.is_some() {
        CodexThreadOrigin::Child
    } else {
        CodexThreadOrigin::Primary
    }))
}

fn find_codex_session_file(dir: &Path, thread_id: &str, depth: usize) -> anyhow::Result<Option<PathBuf>> {
    if depth == 0 || !dir.exists() {
        return Ok(None);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            if let Some(found) = find_codex_session_file(&path, thread_id, depth - 1)? {
                return Ok(Some(found));
            }
        } else if path.extension().is_some_and(|extension| extension == "jsonl")
            && path
                .file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == thread_id || name.ends_with(&format!("-{thread_id}")))
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
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

fn apply_codex_session_index(db: &Db, index: &HashMap<String, String>) -> Result<usize, rusqlite::Error> {
    let transaction = db.conn().unchecked_transaction()?;
    let mut changed = 0;
    for (id, title) in index {
        changed += transaction.execute(
            "UPDATE session_history SET title = ?1 WHERE id = ?2 AND app_type = 'codex' AND title IS NOT ?1",
            rusqlite::params![title, id],
        )?;
    }
    transaction.commit()?;
    Ok(changed)
}

fn parse_codex_session_file(path: &Path) -> Result<Option<SessionRecord>, anyhow::Error> {
    let metadata = std::fs::metadata(path)?;
    let content = std::fs::read_to_string(path)?;
    let Some(canonical) = parse_codex_session_metadata(&content) else {
        return Ok(None);
    };
    let session_id = canonical.id;
    let cwd = canonical.cwd;
    let provider = canonical.provider;
    let parent_thread_id = canonical.parent_thread_id;
    let mut start_time = canonical.start_time.unwrap_or_default();
    let mut end_time = String::new();
    let mut title: Option<String> = None;
    let mut event_message_count = 0i64;
    let mut response_message_count = 0i64;

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
            // The first valid metadata record is canonical. Forked rollouts can
            // contain later metadata copied from their parent.
            "session_meta" => {}
            "event_msg" => {
                let event_type = line.payload.get("type").and_then(serde_json::Value::as_str).unwrap_or("");
                if matches!(event_type, "user_message" | "agent_message") {
                    event_message_count += 1;
                }
                if title.is_none() && event_type == "user_message" {
                    title = line.payload.get("message").and_then(serde_json::Value::as_str).map(truncate_title);
                }
            }
            "response_item" if line.payload.get("type").and_then(serde_json::Value::as_str) == Some("message") => {
                let role = line.payload.get("role").and_then(serde_json::Value::as_str).unwrap_or("");
                if matches!(role, "user" | "assistant") {
                    response_message_count += 1;
                }
                if title.is_none() && role == "user" {
                    title = line
                        .payload
                        .get("content")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|items| {
                            items
                                .iter()
                                .find_map(|item| item.get("text").and_then(serde_json::Value::as_str).filter(|text| !text.trim().is_empty()))
                        })
                        .map(truncate_title);
                }
            }
            _ => {}
        }
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
        parent_thread_id,
        mode: "direct".into(),
        start_time,
        end_time: if end_time.is_empty() { None } else { Some(end_time) },
        prompt_tokens: 0,
        completion_tokens: 0,
        message_count: event_message_count.max(response_message_count),
        search_text: format!("{} {}", title, cwd).to_lowercase(),
        title: Some(title),
        size_bytes: metadata.len() as i64,
        file_mtime,
    }))
}

fn collect_jsonl_files(dir: &Path) -> Vec<PathBuf> {
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

fn parse_session_file(path: &Path, projects_dir: &Path, project_paths: &HashMap<String, PathBuf>) -> Result<Option<SessionRecord>, anyhow::Error> {
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
    let mut recognized_line = false;

    for (i, line) in lines.iter().enumerate() {
        let in_range = i < head_count || i >= lines.len().saturating_sub(tail_count);
        if !in_range {
            if let Some(title) = parse_title_only(line) {
                recognized_line = true;
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
        recognized_line |= parsed.session_id.is_some()
            || parsed.cwd.is_some()
            || parsed.timestamp.is_some()
            || parsed.msg_type.is_some()
            || parsed.message.is_some()
            || parsed.custom_title.is_some()
            || parsed.ai_title.is_some()
            || parsed.last_prompt.is_some();
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

    if !recognized_line {
        return Ok(None);
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
        parent_thread_id: None,
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

fn parse_claude_usage_file(path: &Path, fallback_sid: &str) -> anyhow::Result<Vec<UsageRecord>> {
    let content = std::fs::read_to_string(path)?;
    let mut recognized_line = false;
    let mut records = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let parsed: UsageLine = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        recognized_line |= parsed.msg_type.is_some() || parsed.message.is_some() || parsed.timestamp.is_some() || parsed.session_id.is_some();
        if parsed.msg_type.as_deref() != Some("assistant") {
            continue;
        }
        let Some(msg) = parsed.message.as_ref() else {
            continue;
        };
        let Some(usage) = msg.usage.as_ref() else {
            continue;
        };
        let timestamp = parsed.timestamp.as_deref().unwrap_or("");
        let msg_id = msg.id.clone().unwrap_or_else(|| format!("claude:{}:{}:{}", fallback_sid, timestamp, index));
        let model = msg.ccs_model.as_deref().or(msg.model.as_deref()).unwrap_or("unknown").replace("[1m]", "");
        if model == "<synthetic>" {
            continue;
        }
        let date = normalize_usage_timestamp(timestamp);
        let input = usage.input_tokens.unwrap_or(0).clamp(0, MAX_USAGE_TOKENS);
        let output = usage.output_tokens.unwrap_or(0).clamp(0, MAX_USAGE_TOKENS);
        let cr = usage.cache_read_input_tokens.unwrap_or(0).clamp(0, MAX_USAGE_TOKENS);
        let cc = usage.cache_creation_input_tokens.unwrap_or(0).clamp(0, MAX_USAGE_TOKENS);
        if input == 0 && output == 0 && cr == 0 && cc == 0 {
            continue;
        }
        records.push(UsageRecord {
            msg_id,
            model,
            date,
            input,
            output,
            cr,
            cc,
        });
    }
    anyhow::ensure!(recognized_line, "Usage file contains no recognizable Claude history records");
    Ok(records)
}

fn parse_codex_usage_file(path: &Path, fallback_sid: &str) -> anyhow::Result<(String, Vec<UsageRecord>)> {
    let content = std::fs::read_to_string(path)?;
    let mut sid = fallback_sid.to_string();
    let mut session_meta_seen = false;
    let mut recognized_line = false;
    let mut model = "unknown".to_string();
    let mut records = Vec::new();
    for (index, raw) in content.lines().enumerate() {
        let line: CodexLine = match serde_json::from_str(raw) {
            Ok(line) => line,
            Err(_) => continue,
        };
        recognized_line |= !line.line_type.is_empty();
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
    anyhow::ensure!(recognized_line, "Usage file contains no recognizable Codex history records");
    Ok((sid, records))
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

fn file_mtime(path: &Path) -> Option<i64> {
    let meta = std::fs::metadata(path).ok()?;
    let dur = meta.modified().ok()?;
    let secs = dur.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(secs.as_nanos()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_roots(root: &Path) -> NativeRoots {
        NativeRoots {
            claude_projects: root.join("claude-projects"),
            claude_config: root.join("claude.json"),
            codex_sessions: root.join("codex-sessions"),
            codex_session_index: root.join("codex-session-index.jsonl"),
        }
    }

    fn session_record(id: &str) -> SessionRecord {
        SessionRecord {
            id: id.into(),
            project_path: "/tmp/project".into(),
            profile_id: None,
            parent_thread_id: None,
            mode: "direct".into(),
            start_time: "2026-08-20 10:00:00".into(),
            end_time: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            message_count: 1,
            title: Some("Session".into()),
            size_bytes: 1,
            file_mtime: "2026-08-20 10:00:00".into(),
            search_text: String::new(),
        }
    }

    fn finish_usage_scan(scan: &mut UsageScan, db: &Db) -> UsageScanUpdate {
        for _ in 0..1_000 {
            if let Some(update @ (UsageScanUpdate::Complete(_) | UsageScanUpdate::Failed(_))) = scan.poll(db) {
                return update;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("usage scan did not finish");
    }

    #[test]
    fn ingestion_routes_both_agents_through_the_typed_interface() {
        let root = tempfile::tempdir().unwrap();
        let roots = test_roots(root.path());
        std::fs::create_dir_all(roots.claude_projects.join("project")).unwrap();
        std::fs::create_dir_all(&roots.codex_sessions).unwrap();
        std::fs::write(
            roots.claude_projects.join("project/claude-session.jsonl"),
            r#"{"sessionId":"claude-session","cwd":"/tmp/claude","timestamp":"2026-08-20T10:00:00Z","aiTitle":"Claude title"}"#,
        )
        .unwrap();
        std::fs::write(
            roots.codex_sessions.join("rollout-codex-session.jsonl"),
            r#"{"timestamp":"2026-08-20T10:00:00Z","type":"session_meta","payload":{"id":"codex-session","cwd":"/tmp/codex"}}"#,
        )
        .unwrap();
        let db = Db::open(&root.path().join("ccswitch.db")).unwrap();
        let ingestion = NativeHistoryIngestion::with_roots(&db, roots);

        let claude = ingestion.refresh_sessions(AgentKind::Claude, |_| {}).unwrap();
        let codex = ingestion.refresh_sessions(AgentKind::Codex, |_| {}).unwrap();

        assert_eq!(claude.changed, 1);
        assert_eq!(codex.changed, 1);
        assert_eq!(db.query_sessions("claude", None, None, 10).unwrap()[0].id, "claude-session");
        assert_eq!(db.query_sessions("codex", None, None, 10).unwrap()[0].id, "codex-session");
        let incremental = ingestion.refresh_sessions(AgentKind::Claude, |_| {}).unwrap();
        assert_eq!(incremental.changed, 0);
        assert_eq!(incremental.skipped, 1);
    }

    #[test]
    fn session_record_and_sync_index_commit_atomically() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session.jsonl");
        std::fs::write(&path, r#"{"type":"user","timestamp":"2026-08-20T10:01:00Z"}"#).unwrap();
        let db = Db::open(&root.path().join("ccswitch.db")).unwrap();
        db.conn()
            .execute_batch(
                "CREATE TRIGGER reject_native_history_sync
                 BEFORE INSERT ON session_log_sync
                 BEGIN SELECT RAISE(ABORT, 'sync rejected'); END;",
            )
            .unwrap();

        assert!(persist_session_file(&db, AgentKind::Claude, &session_record("atomic-session"), &path, 1).is_err());
        assert!(db.query_session("claude", "atomic-session").unwrap().is_none());
    }

    #[test]
    fn malformed_changed_file_keeps_the_previous_session_record() {
        let root = tempfile::tempdir().unwrap();
        let roots = test_roots(root.path());
        std::fs::create_dir_all(&roots.codex_sessions).unwrap();
        let path = roots.codex_sessions.join("rollout-stable-session.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        let db = Db::open(&root.path().join("ccswitch.db")).unwrap();
        db.set_setting(CODEX_IMPORT_REVISION_KEY, CODEX_IMPORT_REVISION).unwrap();
        db.insert_session(&session_record("stable-session"), "codex").unwrap();
        db.conn()
            .execute(
                "INSERT INTO session_log_sync (file_path, file_mtime, scan_type) VALUES (?1, 1, 'session')",
                [path.to_string_lossy().as_ref()],
            )
            .unwrap();

        let report = NativeHistoryIngestion::with_roots(&db, roots).refresh_sessions(AgentKind::Codex, |_| {}).unwrap();

        assert_eq!(report.failed, 1);
        assert!(db.query_session("codex", "stable-session").unwrap().is_some());
    }

    #[test]
    fn usage_scan_distinguishes_malformed_files_from_empty_snapshots() {
        let root = tempfile::tempdir().unwrap();
        let roots = test_roots(root.path());
        std::fs::create_dir_all(roots.claude_projects.join("project")).unwrap();
        let path = roots.claude_projects.join("project/session-1.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"assistant","timestamp":"2026-08-20T10:00:00Z","message":{"model":"claude-sonnet","usage":{"input_tokens":10,"output_tokens":2}}}"#,
        )
        .unwrap();
        let db = Db::open(&root.path().join("ccswitch.db")).unwrap();
        let ingestion = NativeHistoryIngestion::with_roots(&db, roots.clone());
        let mut first = ingestion.start_usage_scan(AgentKind::Claude).unwrap();
        assert!(matches!(finish_usage_scan(&mut first, &db), UsageScanUpdate::Complete(_)));
        assert_eq!(db.query_usage("claude", "all").unwrap().len(), 1);

        std::fs::write(&path, "{}\n").unwrap();
        let mut second = ingestion.start_usage_scan(AgentKind::Claude).unwrap();
        let UsageScanUpdate::Complete(malformed) = finish_usage_scan(&mut second, &db) else {
            panic!("malformed usage scan did not complete");
        };
        assert_eq!(malformed.failed, 1);
        assert_eq!(db.query_usage("claude", "all").unwrap().len(), 1);

        std::fs::write(&path, r#"{"type":"user","timestamp":"2026-08-20T10:01:00Z"}"#).unwrap();
        let mut third = ingestion.start_usage_scan(AgentKind::Claude).unwrap();
        assert!(matches!(finish_usage_scan(&mut third, &db), UsageScanUpdate::Complete(_)));
        assert!(db.query_usage("claude", "all").unwrap().is_empty());

        let mut cancelled = ingestion.start_usage_scan(AgentKind::Claude).unwrap();
        cancelled.shutdown();
        assert!(cancelled.handle.is_none());
        assert!(cancelled.terminal);
    }

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

        let roots = NativeRoots {
            claude_projects: projects_dir.clone(),
            ..test_roots(root.path())
        };
        let adapter = ClaudeHistoryAdapter::new(&roots);
        ensure_revision(&db, &adapter, NativeDataset::Sessions).unwrap();
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
        ensure_revision(&db, &adapter, NativeDataset::Sessions).unwrap();
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

        let (sid, records) = parse_codex_usage_file(&path, "fallback").unwrap();
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
    fn counts_codex_response_items_when_event_messages_are_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-response-items.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"timestamp":"2026-08-17T12:00:00Z","type":"session_meta","payload":{"id":"response-session","cwd":"/tmp/project","model_provider":"akmux"}}"#,
                "\n",
                r#"{"timestamp":"2026-08-17T12:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Explain the failure"}]}}"#,
                "\n",
                r#"{"timestamp":"2026-08-17T12:00:02Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"The build order is wrong."}]}}"#,
                "\n",
            ),
        )
        .unwrap();

        let session = parse_codex_session_file(&path).unwrap().unwrap();
        assert_eq!(session.title.as_deref(), Some("Explain the failure"));
        assert_eq!(session.message_count, 2);
    }

    #[test]
    fn codex_child_session_is_hidden_and_aggregated_into_parent() {
        let dir = tempfile::tempdir().unwrap();
        let parent_path = dir.path().join("rollout-parent-session.jsonl");
        let child_path = dir.path().join("rollout-child-session.jsonl");
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
            &child_path,
            concat!(
                r#"{"timestamp":"2026-07-27T08:00:00Z","type":"session_meta","payload":{"id":"fork-session","cwd":"/tmp/fork","model_provider":"fork-provider","parent_thread_id":"parent-session"}}"#,
                "\n",
                r#"{"timestamp":"2026-07-23T08:00:00Z","type":"session_meta","payload":{"id":"parent-session","cwd":"/tmp/parent","model_provider":"parent-provider"}}"#,
                "\n",
                r#"{"timestamp":"2026-07-27T08:00:01Z","type":"turn_context","payload":{"model":"gpt-fork"}}"#,
                "\n",
                r#"{"timestamp":"2026-07-27T08:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":80,"cached_input_tokens":20,"output_tokens":10}}}}"#,
                "\n",
                r#"{"timestamp":"2026-07-27T08:00:03Z","type":"event_msg","payload":{"type":"agent_message","message":"Child result"}}"#,
                "\n"
            ),
        )
        .unwrap();

        let parent = parse_codex_session_file(&parent_path).unwrap().unwrap();
        let child = parse_codex_session_file(&child_path).unwrap().unwrap();
        assert_eq!(child.id, "fork-session");
        assert_eq!(child.parent_thread_id.as_deref(), Some("parent-session"));
        assert_eq!(child.project_path, "/tmp/fork");
        assert_eq!(child.profile_id.as_deref(), Some("fork-provider"));

        let (sid, records) = parse_codex_usage_file(&child_path, "fallback").unwrap();
        assert_eq!(sid, "fork-session");
        assert_eq!(records.len(), 1);
        assert!(records[0].msg_id.starts_with("codex:fork-session:"));

        let db = Db::open(&dir.path().join("ccswitch.db")).unwrap();
        db.insert_session(&parent, "codex").unwrap();
        db.insert_session(&child, "codex").unwrap();
        persist_usage_file(&db, AgentKind::Codex, &sid, &child_path, &records).unwrap();
        let stored = db.query_sessions("codex", None, None, 10).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, "parent-session");
        assert_eq!(stored[0].message_count, 2);
        let all_stored = db.query_all_sessions(None, 10).unwrap();
        assert_eq!(all_stored.len(), 1);
        assert_eq!(all_stored[0].1.id, "parent-session");

        let usage = db.query_session_usage_details("codex", "parent-session").unwrap();
        assert_eq!(usage.prompt_tokens, 60);
        assert_eq!(usage.cache_read_tokens, 20);
        assert_eq!(usage.completion_tokens, 10);
        assert_eq!(db.query_session_tokens("codex", "parent-session").unwrap(), (60, 10));
    }

    #[test]
    fn codex_fork_command_remains_an_independent_session() {
        let dir = tempfile::tempdir().unwrap();
        let parent_path = dir.path().join("rollout-parent-session.jsonl");
        let fork_path = dir.path().join("rollout-fork-session.jsonl");
        std::fs::write(
            &parent_path,
            r#"{"timestamp":"2026-07-23T08:00:00Z","type":"session_meta","payload":{"id":"parent-session","cwd":"/tmp/parent"}}"#,
        )
        .unwrap();
        std::fs::write(
            &fork_path,
            r#"{"timestamp":"2026-07-27T08:00:00Z","type":"session_meta","payload":{"id":"fork-session","cwd":"/tmp/fork","forked_from_id":"parent-session"}}"#,
        )
        .unwrap();

        let parent = parse_codex_session_file(&parent_path).unwrap().unwrap();
        let fork = parse_codex_session_file(&fork_path).unwrap().unwrap();
        assert!(fork.parent_thread_id.is_none());

        let db = Db::open(&dir.path().join("ccswitch.db")).unwrap();
        db.insert_session(&parent, "codex").unwrap();
        db.insert_session(&fork, "codex").unwrap();
        let stored = db.query_sessions("codex", None, None, 10).unwrap();
        assert_eq!(stored.len(), 2);
        assert!(stored.iter().any(|session| session.id == "parent-session"));
        assert!(stored.iter().any(|session| session.id == "fork-session"));
    }

    #[test]
    fn codex_completion_origin_uses_the_canonical_session_metadata() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("2026/08/22")).unwrap();
        std::fs::write(
            dir.path().join("2026/08/22/rollout-root-session.jsonl"),
            r#"{"type":"session_meta","payload":{"id":"root-session","cwd":"/tmp/root"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("2026/08/22/rollout-child-session.jsonl"),
            concat!(
                r#"{"type":"session_meta","payload":{"id":"child-session","cwd":"/tmp/child","parent_thread_id":"root-session"}}"#,
                "\n",
                r#"{"type":"session_meta","payload":{"id":"root-session","cwd":"/tmp/root"}}"#,
            ),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("2026/08/22/rollout-fork-session.jsonl"),
            r#"{"type":"session_meta","payload":{"id":"fork-session","cwd":"/tmp/fork","forked_from_id":"root-session"}}"#,
        )
        .unwrap();

        assert_eq!(codex_thread_origin_in(dir.path(), "root-session").unwrap(), Some(CodexThreadOrigin::Primary));
        assert_eq!(codex_thread_origin_in(dir.path(), "child-session").unwrap(), Some(CodexThreadOrigin::Child));
        assert_eq!(codex_thread_origin_in(dir.path(), "fork-session").unwrap(), Some(CodexThreadOrigin::Primary));
        assert_eq!(codex_thread_origin_in(dir.path(), "missing-session").unwrap(), None);
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
        for (app_type, mode) in [("codex", "direct"), ("claude", "local")] {
            db.conn()
                .execute(
                    "INSERT INTO session_history (id, app_type, project_path, mode, start_time)
                     VALUES (?1, ?2, '/tmp/project', ?3, '2026-07-27 00:00:00')",
                    rusqlite::params![format!("{app_type}-session"), app_type, mode],
                )
                .unwrap();
        }

        let roots = NativeRoots {
            codex_sessions: sessions_dir.clone(),
            ..test_roots(dir.path())
        };
        let adapter = CodexHistoryAdapter::new(&roots);
        ensure_revision(&db, &adapter, NativeDataset::Sessions).unwrap();

        let usage_ids = {
            let mut statement = db.conn().prepare("SELECT message_id FROM usage_logs ORDER BY message_id").unwrap();
            statement.query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert_eq!(usage_ids, vec!["claude-import", "codex-import", "codex-manual"]);
        let session_apps = {
            let mut statement = db.conn().prepare("SELECT app_type FROM session_history ORDER BY app_type").unwrap();
            statement.query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert_eq!(session_apps, vec!["claude"]);

        ensure_revision(&db, &adapter, NativeDataset::Usage).unwrap();

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
        assert_eq!(db.get_setting("native_history_revision:codex:sessions").as_deref(), Some(CODEX_IMPORT_REVISION));
        assert_eq!(db.get_setting("native_history_revision:codex:usage").as_deref(), Some(CODEX_IMPORT_REVISION));

        db.conn()
            .execute(
                "INSERT INTO usage_logs (app_type, model, message_id, data_source) VALUES ('codex', 'model', 'new-import', 'import')",
                [],
            )
            .unwrap();
        ensure_revision(&db, &adapter, NativeDataset::Usage).unwrap();
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

        let records = parse_claude_usage_file(&path, "session-1").unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].msg_id, "claude:session-1:2026-07-27T10:00:00Z:0");
        assert_eq!(records[1].date, "today");
        assert_eq!(records[1].input, MAX_USAGE_TOKENS);

        let reparsed = parse_claude_usage_file(&path, "session-1").unwrap();
        assert_eq!(
            records.iter().map(|record| &record.msg_id).collect::<Vec<_>>(),
            reparsed.iter().map(|record| &record.msg_id).collect::<Vec<_>>()
        );

        let db = Db::open(&dir.path().join("ccswitch.db")).unwrap();
        persist_usage_file(&db, AgentKind::Claude, "session-1", &path, &records).unwrap();
        let replacement = vec![UsageRecord {
            msg_id: records[0].msg_id.clone(),
            model: records[0].model.clone(),
            date: records[0].date.clone(),
            input: 5,
            output: 1,
            cr: 0,
            cc: 0,
        }];
        persist_usage_file(&db, AgentKind::Claude, "session-1", &path, &replacement).unwrap();
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
                parent_thread_id: None,
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
