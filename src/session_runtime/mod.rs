use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex, RwLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::agent::{launch_spec, AgentKind, LaunchMode, LaunchRequest, LaunchSpec};

const OUTPUT_BUFFER_LIMIT: usize = 1024 * 1024;
const EXITED_SESSION_GRACE: Duration = Duration::from_millis(750);
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    fn new() -> Self {
        let millis = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
        let counter = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        Self(format!("session-{millis}-{counter}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Starting,
    Running,
    Exited,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: SessionId,
    pub agent: AgentKind,
    pub title: String,
    pub cwd: PathBuf,
    pub status: SessionStatus,
    pub created_at_ms: u128,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub native_session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateSession {
    pub agent: AgentKind,
    pub title: String,
    pub cwd: PathBuf,
    pub launch_mode: LaunchMode,
    pub rows: u16,
    pub cols: u16,
}

impl CreateSession {
    pub fn new(agent: AgentKind, title: impl Into<String>, cwd: PathBuf) -> Self {
        Self {
            agent,
            title: title.into(),
            cwd,
            launch_mode: LaunchMode::New,
            rows: 24,
            cols: 80,
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        let title = self.title.trim();
        if title.is_empty() {
            anyhow::bail!("Session title must not be empty");
        }
        if title.chars().count() > 128 {
            anyhow::bail!("Session title must be 128 characters or fewer");
        }
        if !self.cwd.is_dir() {
            anyhow::bail!("Working directory does not exist: {}", self.cwd.display());
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum SessionStreamEvent {
    Output(Vec<u8>),
    Status(SessionInfo),
    Attention { kind: AttentionKind, occurred_at_ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttentionKind {
    Input,
    Completed,
}

#[derive(Clone)]
pub struct SessionHandle {
    entry: Arc<SessionEntry>,
}

impl SessionHandle {
    pub fn info(&self) -> SessionInfo {
        read_lock(&self.entry.info).clone()
    }

    pub fn write(&self, bytes: Vec<u8>) -> anyhow::Result<()> {
        self.entry
            .commands
            .send(WorkerCommand::Write(bytes))
            .map_err(|_| anyhow::anyhow!("Session process is not available"))
    }

    pub fn resize(&self, rows: u16, cols: u16) -> anyhow::Result<()> {
        self.entry
            .commands
            .send(WorkerCommand::Resize(TerminalSize::new(rows, cols)))
            .map_err(|_| anyhow::anyhow!("Session process is not available"))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionStreamEvent> {
        self.entry.events.subscribe()
    }

    pub fn scrollback(&self) -> Vec<u8> {
        lock(&self.entry.output).iter().copied().collect()
    }
}

#[derive(Clone)]
pub struct SessionManager {
    inner: Arc<ManagerInner>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self::with_resolver(Arc::new(AgentLaunchResolver))
    }

    fn with_resolver(resolver: Arc<dyn LaunchResolver>) -> Self {
        Self {
            inner: Arc::new(ManagerInner {
                sessions: RwLock::new(HashMap::new()),
                resolver,
            }),
        }
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let mut sessions = read_lock(&self.inner.sessions).values().map(|entry| read_lock(&entry.info).clone()).collect::<Vec<_>>();
        sessions.sort_by_key(|session| session.created_at_ms);
        sessions
    }

    pub fn create(&self, request: CreateSession) -> anyhow::Result<SessionInfo> {
        request.validate()?;
        let id = SessionId::new();
        let native_session_id = match &request.launch_mode {
            LaunchMode::Resume { native_session_id } => Some(native_session_id.clone()),
            LaunchMode::New | LaunchMode::ResumePicker => None,
        };
        let info = SessionInfo {
            id: id.clone(),
            agent: request.agent,
            title: request.title.trim().to_string(),
            cwd: request.cwd.clone(),
            status: SessionStatus::Starting,
            created_at_ms: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
            exit_code: None,
            error: None,
            native_session_id,
        };
        let (commands, command_rx) = mpsc::channel();
        let (events, _) = broadcast::channel(256);
        let entry = Arc::new(SessionEntry {
            info: RwLock::new(info.clone()),
            commands,
            events,
            output: Mutex::new(VecDeque::with_capacity(OUTPUT_BUFFER_LIMIT)),
            last_attention: Mutex::new(None),
            worker: Mutex::new(None),
        });
        write_lock(&self.inner.sessions).insert(id.clone(), Arc::clone(&entry));

        let resolver = Arc::clone(&self.inner.resolver);
        let worker_entry = Arc::clone(&entry);
        let manager = Arc::downgrade(&self.inner);
        let worker_id = id.clone();
        let worker = thread::Builder::new().name(format!("akmux-session-{id}")).spawn(move || {
            if session_worker(Arc::clone(&worker_entry), request, command_rx, resolver) {
                thread::sleep(EXITED_SESSION_GRACE);
                if let Some(manager) = manager.upgrade() {
                    let mut sessions = write_lock(&manager.sessions);
                    if sessions.get(&worker_id).is_some_and(|entry| Arc::ptr_eq(entry, &worker_entry)) {
                        sessions.remove(&worker_id);
                    }
                }
            }
        });
        match worker {
            Ok(worker) => *lock(&entry.worker) = Some(worker),
            Err(error) => {
                write_lock(&self.inner.sessions).remove(&id);
                return Err(error.into());
            }
        }
        Ok(info)
    }

    pub fn get(&self, id: &str) -> Option<SessionHandle> {
        read_lock(&self.inner.sessions)
            .get(&SessionId(id.to_string()))
            .cloned()
            .map(|entry| SessionHandle { entry })
    }

    pub fn restart(&self, id: &str) -> anyhow::Result<()> {
        let session = self.get(id).ok_or_else(|| anyhow::anyhow!("Managed session does not exist"))?;
        session
            .entry
            .commands
            .send(WorkerCommand::Restart)
            .map_err(|_| anyhow::anyhow!("Session process is not available"))
    }

    pub fn close(&self, id: &str) -> anyhow::Result<()> {
        let entry = write_lock(&self.inner.sessions)
            .remove(&SessionId(id.to_string()))
            .ok_or_else(|| anyhow::anyhow!("Managed session does not exist"))?;
        let _ = entry.commands.send(WorkerCommand::Close);
        if let Some(worker) = lock(&entry.worker).take() {
            thread::spawn(move || {
                let _ = worker.join();
            });
        }
        Ok(())
    }

    pub fn update_native_metadata(&self, id: &str, native_session_id: String, title: String) {
        let Some(entry) = read_lock(&self.inner.sessions).get(&SessionId(id.to_string())).cloned() else {
            return;
        };
        let mut info = write_lock(&entry.info);
        if info.native_session_id.as_deref() == Some(&native_session_id) && info.title == title {
            return;
        }
        info.native_session_id = Some(native_session_id);
        info.title = title;
        let _ = entry.events.send(SessionStreamEvent::Status(info.clone()));
    }

    pub fn attention(&self, id: &str, kind: AttentionKind) -> anyhow::Result<()> {
        let session = self.get(id).ok_or_else(|| anyhow::anyhow!("Managed session does not exist"))?;
        let mut last = lock(&session.entry.last_attention);
        if last.is_some_and(|(previous, at)| previous == kind && at.elapsed() < Duration::from_secs(2)) {
            return Ok(());
        }
        *last = Some((kind, Instant::now()));
        let occurred_at_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let _ = session.entry.events.send(SessionStreamEvent::Attention { kind, occurred_at_ms });
        Ok(())
    }

    pub fn shutdown(&self) {
        let entries = write_lock(&self.inner.sessions).drain().map(|(_, entry)| entry).collect::<Vec<_>>();
        for entry in &entries {
            let _ = entry.commands.send(WorkerCommand::Close);
        }
        for entry in entries {
            let worker = lock(&entry.worker).take();
            if let Some(worker) = worker {
                let _ = worker.join();
            }
        }
    }
}

struct ManagerInner {
    sessions: RwLock<HashMap<SessionId, Arc<SessionEntry>>>,
    resolver: Arc<dyn LaunchResolver>,
}

struct SessionEntry {
    info: RwLock<SessionInfo>,
    commands: mpsc::Sender<WorkerCommand>,
    events: broadcast::Sender<SessionStreamEvent>,
    output: Mutex<VecDeque<u8>>,
    last_attention: Mutex<Option<(AttentionKind, Instant)>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug, Clone, Copy)]
struct TerminalSize {
    rows: u16,
    cols: u16,
}

impl TerminalSize {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            rows: rows.max(1),
            cols: cols.max(1),
        }
    }

    fn as_pty(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

enum WorkerCommand {
    Write(Vec<u8>),
    Resize(TerminalSize),
    Restart,
    Close,
}

enum ProcessOutcome {
    Restart,
    Closed,
    Exited,
    Failed,
}

trait LaunchResolver: Send + Sync {
    fn resolve(&self, request: &CreateSession, managed_session_id: &str) -> anyhow::Result<LaunchSpec>;
}

struct AgentLaunchResolver;

impl LaunchResolver for AgentLaunchResolver {
    fn resolve(&self, request: &CreateSession, managed_session_id: &str) -> anyhow::Result<LaunchSpec> {
        launch_spec(&LaunchRequest {
            agent: request.agent,
            cwd: request.cwd.clone(),
            mode: request.launch_mode.clone(),
            managed_session_id: Some(managed_session_id.to_string()),
        })
    }
}

fn session_worker(entry: Arc<SessionEntry>, request: CreateSession, command_rx: mpsc::Receiver<WorkerCommand>, resolver: Arc<dyn LaunchResolver>) -> bool {
    let mut size = TerminalSize::new(request.rows, request.cols);
    loop {
        set_status(&entry, SessionStatus::Starting, None, None);
        let managed_session_id = read_lock(&entry.info).id.to_string();
        let outcome = match resolver.resolve(&request, &managed_session_id) {
            Ok(spec) => run_process(&entry, &spec, &command_rx, &mut size),
            Err(error) => {
                set_status(&entry, SessionStatus::Error, None, Some(error.to_string()));
                ProcessOutcome::Failed
            }
        };
        match outcome {
            ProcessOutcome::Restart => continue,
            ProcessOutcome::Closed => return false,
            ProcessOutcome::Exited => return true,
            ProcessOutcome::Failed => loop {
                match command_rx.recv() {
                    Ok(WorkerCommand::Restart) => break,
                    Ok(WorkerCommand::Resize(new_size)) => size = new_size,
                    Ok(WorkerCommand::Close) | Err(_) => return false,
                    Ok(WorkerCommand::Write(_)) => {}
                }
            },
        }
    }
}

fn run_process(entry: &Arc<SessionEntry>, spec: &LaunchSpec, command_rx: &mpsc::Receiver<WorkerCommand>, size: &mut TerminalSize) -> ProcessOutcome {
    match run_process_inner(entry, spec, command_rx, size) {
        Ok(outcome) => outcome,
        Err(error) => {
            set_status(entry, SessionStatus::Error, None, Some(error.to_string()));
            ProcessOutcome::Failed
        }
    }
}

fn run_process_inner(entry: &Arc<SessionEntry>, spec: &LaunchSpec, command_rx: &mpsc::Receiver<WorkerCommand>, size: &mut TerminalSize) -> anyhow::Result<ProcessOutcome> {
    let pair = native_pty_system().openpty(size.as_pty())?;
    let mut command = CommandBuilder::new(&spec.program);
    command.args(&spec.args);
    command.cwd(&spec.cwd);
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    configure_terminal_environment(&mut command);
    let mut child = pair.slave.spawn_command(command)?;
    drop(pair.slave);
    let master = pair.master;
    let reader = master.try_clone_reader()?;
    let mut writer = master.take_writer()?;
    let reader_cancel = Arc::new(AtomicBool::new(false));
    let reader_entry = Arc::clone(entry);
    let cancel = Arc::clone(&reader_cancel);
    let reader_thread = thread::Builder::new()
        .name(format!("akmux-pty-reader-{}", read_lock(&entry.info).id))
        .spawn(move || read_pty(reader, reader_entry, cancel))?;
    let reader = PtyReaderHandle {
        cancel: reader_cancel,
        thread: reader_thread,
    };

    set_status(entry, SessionStatus::Running, None, None);
    loop {
        match command_rx.recv_timeout(Duration::from_millis(40)) {
            Ok(WorkerCommand::Write(bytes)) => {
                writer.write_all(&bytes)?;
                writer.flush()?;
            }
            Ok(WorkerCommand::Resize(new_size)) => {
                *size = new_size;
                master.resize(new_size.as_pty())?;
            }
            Ok(WorkerCommand::Restart) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(writer);
                drop(master);
                reader.stop(Duration::ZERO);
                return Ok(ProcessOutcome::Restart);
            }
            Ok(WorkerCommand::Close) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(writer);
                drop(master);
                reader.stop(Duration::ZERO);
                return Ok(ProcessOutcome::Closed);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if let Some(status) = child.try_wait()? {
            let code = Some(status.exit_code() as i32);
            set_status(entry, SessionStatus::Exited, code, None);
            drop(writer);
            drop(master);
            reader.stop(Duration::from_millis(20));
            return Ok(ProcessOutcome::Exited);
        }
    }
}

fn configure_terminal_environment(command: &mut CommandBuilder) {
    command.env_remove("NO_COLOR");
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("COLORFGBG", "15;0");
    command.env("TERM_PROGRAM", "akmux");
    command.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
}

struct PtyReaderHandle {
    cancel: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl PtyReaderHandle {
    fn stop(self, grace: Duration) {
        let deadline = Instant::now() + grace;
        while !self.thread.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        self.cancel.store(true, Ordering::Release);
        if self.thread.is_finished() {
            let _ = self.thread.join();
        }
    }
}

fn read_pty(mut reader: Box<dyn Read + Send>, entry: Arc<SessionEntry>, cancel: Arc<AtomicBool>) {
    let mut buffer = [0u8; 8192];
    loop {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let read = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        if cancel.load(Ordering::Acquire) {
            break;
        }
        append_output(&entry, &buffer[..read]);
        let _ = entry.events.send(SessionStreamEvent::Output(buffer[..read].to_vec()));
    }
}

fn append_output(entry: &SessionEntry, bytes: &[u8]) {
    let mut output = lock(&entry.output);
    if bytes.len() >= OUTPUT_BUFFER_LIMIT {
        output.clear();
        output.extend(bytes[bytes.len() - OUTPUT_BUFFER_LIMIT..].iter().copied());
        return;
    }
    let overflow = output.len().saturating_add(bytes.len()).saturating_sub(OUTPUT_BUFFER_LIMIT);
    output.drain(..overflow);
    output.extend(bytes.iter().copied());
}

fn set_status(entry: &SessionEntry, status: SessionStatus, exit_code: Option<i32>, error: Option<String>) {
    let info = {
        let mut info = write_lock(&entry.info);
        info.status = status;
        info.exit_code = exit_code;
        info.error = error;
        info.clone()
    };
    let _ = entry.events.send(SessionStreamEvent::Status(info));
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{configure_terminal_environment, CreateSession, LaunchResolver, SessionManager, SessionStatus};
    use crate::agent::{AgentKind, LaunchSpec};
    use portable_pty::CommandBuilder;
    use std::{
        path::PathBuf,
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    struct FixedResolver {
        spec: LaunchSpec,
    }

    impl LaunchResolver for FixedResolver {
        fn resolve(&self, _request: &CreateSession, _managed_session_id: &str) -> anyhow::Result<LaunchSpec> {
            Ok(self.spec.clone())
        }
    }

    #[test]
    fn rejects_missing_working_directory() {
        let manager = SessionManager::new();
        let error = manager
            .create(CreateSession::new(AgentKind::Codex, "test", PathBuf::from("/definitely/missing/ccswitch")))
            .unwrap_err();
        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn managed_pty_advertises_truecolor_and_ignores_no_color() {
        let mut command = CommandBuilder::new("fixture");
        command.env("NO_COLOR", "1");

        configure_terminal_environment(&mut command);

        assert_eq!(command.get_env("TERM"), Some(std::ffi::OsStr::new("xterm-256color")));
        assert_eq!(command.get_env("COLORTERM"), Some(std::ffi::OsStr::new("truecolor")));
        assert_eq!(command.get_env("COLORFGBG"), Some(std::ffi::OsStr::new("15;0")));
        assert_eq!(command.get_env("NO_COLOR"), None);
    }

    #[cfg(unix)]
    #[test]
    fn routes_pty_input_and_output() {
        let cwd = std::env::current_dir().unwrap();
        let manager = SessionManager::with_resolver(Arc::new(FixedResolver {
            spec: LaunchSpec {
                program: "sh".into(),
                args: vec!["-c".into(), "printf ready; read line; printf 'got:%s' \"$line\"".into()],
                cwd: cwd.clone(),
                env: Vec::new(),
            },
        }));
        let info = manager.create(CreateSession::new(AgentKind::Codex, "fixture", cwd)).unwrap();
        let session = manager.get(info.id.as_str()).unwrap();
        wait_until(Duration::from_secs(3), || String::from_utf8_lossy(&session.scrollback()).contains("ready"));
        session.write(b"hello\r".to_vec()).unwrap();
        wait_until(Duration::from_secs(3), || String::from_utf8_lossy(&session.scrollback()).contains("got:hello"));
        wait_until(Duration::from_secs(3), || session.info().status == SessionStatus::Exited);
        manager.shutdown();
    }

    #[cfg(unix)]
    #[test]
    fn removes_sessions_after_the_process_exits() {
        let cwd = std::env::current_dir().unwrap();
        let manager = SessionManager::with_resolver(Arc::new(FixedResolver {
            spec: LaunchSpec {
                program: "sh".into(),
                args: vec!["-c".into(), "exit 0".into()],
                cwd: cwd.clone(),
                env: Vec::new(),
            },
        }));

        manager.create(CreateSession::new(AgentKind::Codex, "fixture", cwd)).unwrap();
        wait_until(Duration::from_secs(3), || manager.list().is_empty());
        manager.shutdown();
    }

    fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("condition was not met within {timeout:?}");
    }
}
