use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    sync::{Arc, Mutex},
};

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, Request, State,
    },
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::{
    agent::{AgentKind, LaunchMode},
    core::{config, import},
    db::{sessions::SessionRecord, Db},
    session_runtime::{CreateSession, SessionHandle, SessionInfo, SessionManager, SessionStreamEvent},
};

pub mod control;

const DEFAULT_PORT: u16 = 17321;
const INDEX_HTML: &str = include_str!("../../web/session-ui/dist/index.html");
const APP_JS: &[u8] = include_bytes!("../../web/session-ui/dist/app.js");
const STYLE_CSS: &[u8] = include_bytes!("../../web/session-ui/dist/style.css");
const AKIRON_ICON: &[u8] = include_bytes!("../../web/session-ui/dist/akiron.svg");
const OPENAI_ICON: &[u8] = include_bytes!("../../web/session-ui/dist/openai.svg");
const CLAUDE_ICON: &[u8] = include_bytes!("../../web/session-ui/dist/claude.svg");
const MAPLE_MONO_REGULAR: &[u8] = include_bytes!("../../web/session-ui/dist/fonts/MapleMonoNormalNL-NF-Regular.woff2");
const MAPLE_MONO_BOLD: &[u8] = include_bytes!("../../web/session-ui/dist/fonts/MapleMonoNormalNL-NF-Bold.woff2");
const MAPLE_MONO_CN: &[u8] = include_bytes!("../../web/session-ui/dist/fonts/MapleMonoNormalNL-NF-CN-Medium.woff2");
const MAPLE_MONO_LICENSE: &[u8] = include_bytes!("../../web/session-ui/dist/fonts/OFL.txt");

#[derive(Clone)]
struct AppState {
    manager: SessionManager,
    db: Arc<Mutex<Db>>,
    workspaces: Arc<Mutex<WorkspaceState>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Project {
    id: String,
    name: String,
    path: String,
    pinned: bool,
    sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceDirectory {
    path: String,
    pinned: bool,
    last_opened_ms: i64,
    #[serde(default)]
    sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceState {
    general_root: String,
    projects: Vec<Project>,
    other_directories: Vec<WorkspaceDirectory>,
    project_sort: SortMode,
    general_sort: SortMode,
    other_sort: SortMode,
    #[serde(default)]
    directory_sort: HashMap<String, SortMode>,
    #[serde(default)]
    session_order: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SortMode {
    Priority,
    Recent,
    Manual,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            general_root: default_general_root().display().to_string(),
            projects: Vec::new(),
            other_directories: Vec::new(),
            project_sort: SortMode::Priority,
            general_sort: SortMode::Recent,
            other_sort: SortMode::Recent,
            directory_sort: std::collections::HashMap::new(),
            session_order: std::collections::HashMap::new(),
        }
    }
}

fn default_general_root() -> PathBuf {
    let home = default_working_directory();
    let workbench = home.join("workbench");
    if workbench.is_dir() {
        workbench
    } else {
        home
    }
}

#[derive(Debug, Clone, Serialize)]
struct HistoryItem {
    id: String,
    agent: AgentKind,
    title: String,
    cwd: String,
    start_time: String,
    end_time: Option<String>,
    file_mtime: String,
    message_count: i64,
}

#[derive(Debug, Clone, Serialize)]
struct SessionDetails {
    managed_session_id: String,
    native_session_id: Option<String>,
    agent: AgentKind,
    provider_id: Option<String>,
    provider_name: Option<String>,
    profile_id: Option<String>,
    model: Option<String>,
    prompt_tokens: i64,
    completion_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    message_count: i64,
}

#[derive(Debug, Clone, Serialize)]
struct HistoryDirectory {
    path: String,
    available: bool,
    items: Vec<HistoryItem>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkspaceResponse {
    general_root: String,
    projects: Vec<ProjectGroup>,
    general: Vec<HistoryDirectory>,
    other: Vec<HistoryDirectory>,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectGroup {
    project: Project,
    history: Vec<HistoryItem>,
}

#[derive(Debug, Deserialize)]
struct ProjectRequest {
    path: String,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProjectPatch {
    name: Option<String>,
    path: Option<String>,
    pinned: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct WorkspacePatch {
    general_root: Option<String>,
    project_sort: Option<SortMode>,
    general_sort: Option<SortMode>,
    other_sort: Option<SortMode>,
    directory_sort: Option<DirectorySortPatch>,
}

#[derive(Debug, Deserialize)]
struct DirectorySortPatch {
    path: String,
    mode: SortMode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReorderKind {
    Projects,
    Directories,
    Sessions,
}

#[derive(Debug, Deserialize)]
struct ReorderRequest {
    kind: ReorderKind,
    scope: String,
    ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CreateSessionRequest {
    agent: AgentKind,
    #[serde(default)]
    title: String,
    cwd: PathBuf,
    #[serde(default = "default_rows")]
    rows: u16,
    #[serde(default = "default_cols")]
    cols: u16,
    #[serde(default)]
    resume: bool,
    resume_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientControl {
    Resize { rows: u16, cols: u16 },
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    default_cwd: String,
}

#[derive(Debug, Deserialize)]
struct DirectoryQuery {
    path: Option<String>,
    #[serde(default)]
    show_hidden: bool,
}

#[derive(Debug, Deserialize)]
struct CreateDirectoryRequest {
    parent: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct DirectoryListing {
    path: String,
    parent: Option<String>,
    home: Option<String>,
    entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Serialize)]
struct DirectoryEntry {
    name: String,
    path: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

pub async fn run_from_env() -> anyhow::Result<()> {
    if !control::configured_enabled()? {
        tracing::info!("AkironMux session service is disabled in TUI settings");
        return Ok(());
    }
    let port = std::env::var("AKMUX_SESSION_PORT")
        .or_else(|_| std::env::var("CCSWITCH_SESSION_PORT"))
        .ok()
        .map(|value| value.parse::<u16>())
        .transpose()?
        .unwrap_or(DEFAULT_PORT);
    run(port).await
}

pub async fn run(port: u16) -> anyhow::Result<()> {
    let manager = SessionManager::new();
    let db = Db::open(&config::db_path())?;
    if let Err(error) = refresh_native_history(&db) {
        tracing::warn!("Native history refresh failed during service startup: {error:#}");
    }
    let app = router_with_db(manager.clone(), db);
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .map_err(|error| anyhow::anyhow!("Failed to bind session service on 127.0.0.1:{port}: {error}"))?;
    let address = listener.local_addr()?;
    if let Err(error) = write_service_state(address) {
        tracing::warn!("Unable to write service state file: {error:#}");
    }
    tracing::info!("AkironMux session service listening on http://{}", address);

    let result = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await;
    manager.shutdown();
    let _ = std::fs::remove_file(service_state_path());
    result?;
    Ok(())
}

#[allow(dead_code)]
fn router(manager: SessionManager) -> Router {
    let db = Db::open(FsPath::new(":memory:")).expect("in-memory session service database");
    router_with_db(manager, db)
}

fn router_with_db(manager: SessionManager, db: Db) -> Router {
    let workspace = load_workspace(&db);
    let state = Arc::new(AppState {
        manager,
        db: Arc::new(Mutex::new(db)),
        workspaces: Arc::new(Mutex::new(workspace)),
    });
    start_title_sync(Arc::clone(&state));
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/style.css", get(style_css))
        .route("/akiron.svg", get(akiron_icon))
        .route("/openai.svg", get(openai_icon))
        .route("/claude.svg", get(claude_icon))
        .route("/fonts/MapleMonoNormalNL-NF-Regular.woff2", get(maple_mono_regular))
        .route("/fonts/MapleMonoNormalNL-NF-Bold.woff2", get(maple_mono_bold))
        .route("/fonts/MapleMonoNormalNL-NF-CN-Medium.woff2", get(maple_mono_cn))
        .route("/fonts/OFL.txt", get(maple_mono_license))
        .route("/favicon.ico", get(|| async { StatusCode::NO_CONTENT }))
        .route("/api/health", get(health))
        .route("/api/directories", get(list_directories).post(create_directory))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/:id/details", get(session_details))
        .route("/api/sessions/:id/restart", post(restart_session))
        .route("/api/sessions/:id", delete(close_session))
        .route("/api/sessions/:id/terminal", get(terminal_websocket))
        .route("/api/workspaces", get(workspaces))
        .route("/api/projects", post(create_project))
        .route("/api/projects/:id", axum::routing::patch(update_project).delete(delete_project))
        .route("/api/history", get(history))
        .route("/api/history/refresh", post(refresh_history))
        .route("/api/settings", get(settings).patch(update_settings))
        .route("/api/reorder", post(reorder_workspace_items))
        .with_state(state)
        .layer(middleware::from_fn(validate_local_request))
}

fn start_title_sync(state: Arc<AppState>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            let sync_state = Arc::clone(&state);
            let _ = tokio::task::spawn_blocking(move || sync_native_titles(&sync_state)).await;
        }
    });
}

fn sync_native_titles(state: &AppState) -> anyhow::Result<()> {
    let records = {
        let db = state.db.lock().map_err(|_| anyhow::anyhow!("Database lock poisoned"))?;
        import::refresh_session_titles(&db)?;
        db.query_all_sessions(None, 2000)?
    };
    for session in state.manager.list() {
        let matched = if let Some(native_id) = session.native_session_id.as_deref() {
            records.iter().find(|(app_type, record)| agent_matches(session.agent, app_type) && record.id == native_id)
        } else {
            records
                .iter()
                .filter(|(app_type, record)| agent_matches(session.agent, app_type) && FsPath::new(&record.project_path) == session.cwd.as_path())
                .filter(|(_, record)| history_time_ms(&record.file_mtime).is_some_and(|time| time + 5_000 >= session.created_at_ms as i64))
                .max_by_key(|(_, record)| record.file_mtime.clone())
        };
        if let Some((_, record)) = matched {
            if let Some(title) = record.title.as_ref() {
                state.manager.update_native_metadata(session.id.as_str(), record.id.clone(), title.clone());
            }
        }
    }
    Ok(())
}

fn agent_matches(agent: AgentKind, app_type: &str) -> bool {
    matches!((agent, app_type), (AgentKind::Claude, "claude") | (AgentKind::Codex, "codex"))
}

fn history_time_ms(value: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|time| time.and_utc().timestamp_millis())
}

async fn validate_local_request(request: Request, next: Next) -> Response {
    let headers = request.headers();
    let valid_host = headers.get(header::HOST).and_then(|value| value.to_str().ok()).is_some_and(is_local_authority);
    let origin = headers.get(header::ORIGIN).and_then(|value| value.to_str().ok()).map(str::to_owned);
    let valid_origin = origin.as_deref().map_or(true, is_local_origin);
    if !valid_host || !valid_origin {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Only local browser clients are allowed".into(),
            }),
        )
            .into_response();
    }
    let preflight = request.method() == Method::OPTIONS;
    let mut response = if preflight { StatusCode::NO_CONTENT.into_response() } else { next.run(request).await };
    if let Some(origin) = origin.and_then(|origin| HeaderValue::from_str(&origin).ok()) {
        response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        response.headers_mut().insert(header::VARY, HeaderValue::from_static("Origin"));
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, PATCH, DELETE, OPTIONS"));
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("Content-Type"));
    }
    response
}

fn is_local_authority(authority: &str) -> bool {
    authority == "localhost"
        || authority.starts_with("localhost:")
        || authority == "127.0.0.1"
        || authority.starts_with("127.0.0.1:")
        || authority == "[::1]"
        || authority.starts_with("[::1]:")
}

fn is_local_origin(origin: &str) -> bool {
    if matches!(origin, "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost") {
        return true;
    }
    let Some(authority) = origin.strip_prefix("http://").or_else(|| origin.strip_prefix("https://")) else {
        return false;
    };
    is_local_authority(authority.split('/').next().unwrap_or_default())
}

async fn index() -> Response {
    static_response(INDEX_HTML.as_bytes(), "text/html; charset=utf-8", true)
}

async fn app_js() -> Response {
    static_response(APP_JS, "text/javascript; charset=utf-8", false)
}

async fn style_css() -> Response {
    static_response(STYLE_CSS, "text/css; charset=utf-8", false)
}

async fn akiron_icon() -> Response {
    static_response(AKIRON_ICON, "image/svg+xml", false)
}

async fn openai_icon() -> Response {
    static_response(OPENAI_ICON, "image/svg+xml", false)
}

async fn claude_icon() -> Response {
    static_response(CLAUDE_ICON, "image/svg+xml", false)
}

async fn maple_mono_regular() -> Response {
    static_response(MAPLE_MONO_REGULAR, "font/woff2", false)
}

async fn maple_mono_bold() -> Response {
    static_response(MAPLE_MONO_BOLD, "font/woff2", false)
}

async fn maple_mono_cn() -> Response {
    static_response(MAPLE_MONO_CN, "font/woff2", false)
}

async fn maple_mono_license() -> Response {
    static_response(MAPLE_MONO_LICENSE, "text/plain; charset=utf-8", false)
}

fn static_response(body: &[u8], content_type: &'static str, no_store: bool) -> Response {
    let mut response = Response::new(Body::from(body.to_vec()));
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    response.headers_mut().insert("x-frame-options", HeaderValue::from_static("DENY"));
    response.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; connect-src 'self' ws://127.0.0.1:* ws://localhost:*; style-src 'self' 'unsafe-inline'; script-src 'self'; img-src 'self' data:",
        ),
    );
    if no_store {
        response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        default_cwd: default_working_directory().display().to_string(),
    })
}

fn refresh_native_history(db: &Db) -> anyhow::Result<()> {
    let _ = import::import_claude_sessions(db)?;
    let _ = import::import_codex_sessions(db)?;
    Ok(())
}

fn load_workspace(db: &Db) -> WorkspaceState {
    db.get_setting("akironmux.workspace")
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

fn save_workspace(db: &Db, workspace: &WorkspaceState) -> anyhow::Result<()> {
    db.set_setting("akironmux.workspace", &serde_json::to_string(workspace)?)?;
    Ok(())
}

fn canonical_directory(path: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(path);
    if path.as_os_str().len() > 4096 {
        anyhow::bail!("Directory path is too long");
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("Cannot open directory '{}': {error}", path.display()))?;
    if !canonical.is_dir() {
        anyhow::bail!("Path is not a directory: {}", canonical.display());
    }
    Ok(canonical)
}

fn paths_overlap(left: &FsPath, right: &FsPath) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn canonicalize_for_comparison(path: &FsPath) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn workspace_response(db: &Db, workspace: &mut WorkspaceState, search: Option<&str>) -> anyhow::Result<WorkspaceResponse> {
    let records = db.query_all_sessions(search, 2000)?;
    let mut projects = workspace
        .projects
        .iter()
        .cloned()
        .map(|project| ProjectGroup { project, history: Vec::new() })
        .collect::<Vec<_>>();
    let project_roots = projects
        .iter()
        .map(|group| canonicalize_for_comparison(FsPath::new(&group.project.path)))
        .collect::<Vec<_>>();
    let mut general: HashMap<String, HistoryDirectory> = HashMap::new();
    let mut other: HashMap<String, HistoryDirectory> = HashMap::new();
    let general_root = canonicalize_for_comparison(FsPath::new(&workspace.general_root));

    for (app_type, record) in records {
        if record.project_path.trim().is_empty() {
            continue;
        }
        let cwd = PathBuf::from(&record.project_path);
        let item = HistoryItem {
            id: record.id,
            agent: if app_type == "claude" { AgentKind::Claude } else { AgentKind::Codex },
            title: record
                .title
                .unwrap_or_else(|| cwd.file_name().and_then(|name| name.to_str()).unwrap_or("Session").to_string()),
            cwd: record.project_path.clone(),
            start_time: record.start_time,
            end_time: record.end_time,
            file_mtime: record.file_mtime,
            message_count: record.message_count,
        };
        let comparison_cwd = canonicalize_for_comparison(&cwd);
        let project_index = project_roots.iter().position(|root| comparison_cwd.starts_with(root));
        if let Some(index) = project_index {
            projects[index].history.push(item);
        } else if comparison_cwd == general_root || comparison_cwd.starts_with(&general_root) {
            let key = cwd.display().to_string();
            general
                .entry(key.clone())
                .or_insert_with(|| HistoryDirectory {
                    path: key,
                    available: cwd.is_dir(),
                    items: Vec::new(),
                })
                .items
                .push(item);
        } else {
            let key = cwd.display().to_string();
            other
                .entry(key.clone())
                .or_insert_with(|| HistoryDirectory {
                    path: key,
                    available: cwd.is_dir(),
                    items: Vec::new(),
                })
                .items
                .push(item);
        }
    }

    let now = chrono::Utc::now().timestamp_millis();
    let visible_directories = general.keys().chain(other.keys()).cloned().collect::<Vec<_>>();
    for path in visible_directories {
        if workspace.other_directories.iter().all(|directory| directory.path != path) {
            workspace.other_directories.push(WorkspaceDirectory {
                path,
                pinned: false,
                last_opened_ms: now,
                sort_order: workspace.other_directories.len() as i64,
            });
        }
    }

    projects.sort_by_key(|group| group.project.sort_order);
    for group in &mut projects {
        let scope = format!("project:{}", group.project.id);
        sort_history(&mut group.history, workspace.project_sort, workspace.session_order.get(&scope));
    }
    let mut general = sort_directories(general, &workspace.other_directories);
    for group in &mut general {
        let mode = workspace.directory_sort.get(&group.path).copied().unwrap_or(workspace.general_sort);
        let scope = format!("directory:{}", group.path);
        sort_history(&mut group.items, mode, workspace.session_order.get(&scope));
    }
    let mut other = sort_directories(other, &workspace.other_directories);
    for group in &mut other {
        let mode = workspace.directory_sort.get(&group.path).copied().unwrap_or(workspace.other_sort);
        let scope = format!("directory:{}", group.path);
        sort_history(&mut group.items, mode, workspace.session_order.get(&scope));
    }
    Ok(WorkspaceResponse {
        general_root: workspace.general_root.clone(),
        projects,
        general,
        other,
    })
}

fn sort_history(items: &mut [HistoryItem], mode: SortMode, manual_order: Option<&Vec<String>>) {
    match mode {
        SortMode::Priority => items.sort_by_key(|item| (std::cmp::Reverse(item.message_count), std::cmp::Reverse(item.file_mtime.clone()))),
        SortMode::Recent => items.sort_by_key(|item| std::cmp::Reverse(item.file_mtime.clone())),
        SortMode::Manual => {
            let positions = manual_order
                .map(|order| order.iter().enumerate().map(|(index, id)| (id.as_str(), index)).collect::<HashMap<_, _>>())
                .unwrap_or_default();
            items.sort_by_key(|item| {
                let agent = match item.agent {
                    AgentKind::Claude => "claude",
                    AgentKind::Codex => "codex",
                };
                positions.get(format!("{agent}:{}", item.id).as_str()).copied().unwrap_or(usize::MAX)
            });
        }
    }
}

fn sort_directories(groups: HashMap<String, HistoryDirectory>, metadata: &[WorkspaceDirectory]) -> Vec<HistoryDirectory> {
    let mut values = groups.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        let left_meta = metadata.iter().find(|entry| entry.path == left.path);
        let right_meta = metadata.iter().find(|entry| entry.path == right.path);
        left_meta
            .map(|entry| entry.sort_order)
            .unwrap_or(i64::MAX)
            .cmp(&right_meta.map(|entry| entry.sort_order).unwrap_or(i64::MAX))
            .then_with(|| left.path.cmp(&right.path))
    });
    values
}

async fn workspaces(State(state): State<Arc<AppState>>, Query(query): Query<SearchQuery>) -> Result<Json<WorkspaceResponse>, ApiError> {
    let db = state.db.lock().map_err(|_| ApiError::internal("Database lock poisoned"))?;
    let mut workspace = state.workspaces.lock().map_err(|_| ApiError::internal("Workspace lock poisoned"))?;
    let response = workspace_response(&db, &mut workspace, query.q.as_deref()).map_err(ApiError::bad_request)?;
    save_workspace(&db, &workspace).map_err(ApiError::bad_request)?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

async fn history(State(state): State<Arc<AppState>>, Query(query): Query<SearchQuery>) -> Result<Json<Vec<HistoryItem>>, ApiError> {
    let db = state.db.lock().map_err(|_| ApiError::internal("Database lock poisoned"))?;
    let records = db.query_all_sessions(query.q.as_deref(), 2000).map_err(|error| ApiError::bad_request(error.into()))?;
    Ok(Json(records.into_iter().map(|(app_type, record)| history_item(app_type, record)).collect()))
}

fn history_item(app_type: String, record: SessionRecord) -> HistoryItem {
    let cwd = PathBuf::from(&record.project_path);
    HistoryItem {
        id: record.id,
        agent: if app_type == "claude" { AgentKind::Claude } else { AgentKind::Codex },
        title: record
            .title
            .unwrap_or_else(|| cwd.file_name().and_then(|name| name.to_str()).unwrap_or("Session").to_string()),
        cwd: record.project_path,
        start_time: record.start_time,
        end_time: record.end_time,
        file_mtime: record.file_mtime,
        message_count: record.message_count,
    }
}

async fn refresh_history(State(state): State<Arc<AppState>>) -> Result<Json<WorkspaceResponse>, ApiError> {
    let db = state.db.lock().map_err(|_| ApiError::internal("Database lock poisoned"))?;
    refresh_native_history(&db).map_err(ApiError::bad_request)?;
    let mut workspace = state.workspaces.lock().map_err(|_| ApiError::internal("Workspace lock poisoned"))?;
    let response = workspace_response(&db, &mut workspace, None).map_err(ApiError::bad_request)?;
    save_workspace(&db, &workspace).map_err(ApiError::bad_request)?;
    Ok(Json(response))
}

async fn create_project(State(state): State<Arc<AppState>>, Json(request): Json<ProjectRequest>) -> Result<(StatusCode, Json<Project>), ApiError> {
    let path = canonical_directory(&request.path).map_err(ApiError::bad_request)?;
    let db = state.db.lock().map_err(|_| ApiError::internal("Database lock poisoned"))?;
    let mut workspace = state.workspaces.lock().map_err(|_| ApiError::internal("Workspace lock poisoned"))?;
    let general = PathBuf::from(&workspace.general_root);
    if paths_overlap(&path, &general) || workspace.projects.iter().any(|project| paths_overlap(&path, FsPath::new(&project.path))) {
        return Err(ApiError::bad_request(anyhow::anyhow!("Project directory overlaps an existing workspace")));
    }
    let project = Project {
        id: format!("project-{}", chrono::Utc::now().timestamp_millis()),
        name: request
            .name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| path.file_name().and_then(|name| name.to_str()).unwrap_or("Project").to_string()),
        path: path.display().to_string(),
        pinned: false,
        sort_order: workspace.projects.len() as i64,
    };
    workspace.projects.push(project.clone());
    save_workspace(&db, &workspace).map_err(ApiError::bad_request)?;
    Ok((StatusCode::CREATED, Json(project)))
}

async fn update_project(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(request): Json<ProjectPatch>) -> Result<Json<Project>, ApiError> {
    let db = state.db.lock().map_err(|_| ApiError::internal("Database lock poisoned"))?;
    let mut workspace = state.workspaces.lock().map_err(|_| ApiError::internal("Workspace lock poisoned"))?;
    let index = workspace
        .projects
        .iter()
        .position(|project| project.id == id)
        .ok_or_else(|| ApiError::not_found(anyhow::anyhow!("Project does not exist")))?;
    if let Some(path) = request.path {
        let path = canonical_directory(&path).map_err(ApiError::bad_request)?;
        let general = PathBuf::from(&workspace.general_root);
        if paths_overlap(&path, &general)
            || workspace
                .projects
                .iter()
                .enumerate()
                .any(|(other, project)| other != index && paths_overlap(&path, FsPath::new(&project.path)))
        {
            return Err(ApiError::bad_request(anyhow::anyhow!("Project directory overlaps an existing workspace")));
        }
        workspace.projects[index].path = path.display().to_string();
    }
    if let Some(name) = request.name.filter(|name| !name.trim().is_empty()) {
        workspace.projects[index].name = name.trim().to_string();
    }
    if let Some(pinned) = request.pinned {
        workspace.projects[index].pinned = pinned;
    }
    let project = workspace.projects[index].clone();
    save_workspace(&db, &workspace).map_err(ApiError::bad_request)?;
    Ok(Json(project))
}

async fn delete_project(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let db = state.db.lock().map_err(|_| ApiError::internal("Database lock poisoned"))?;
    let mut workspace = state.workspaces.lock().map_err(|_| ApiError::internal("Workspace lock poisoned"))?;
    let before = workspace.projects.len();
    workspace.projects.retain(|project| project.id != id);
    if workspace.projects.len() == before {
        return Err(ApiError::not_found(anyhow::anyhow!("Project does not exist")));
    }
    save_workspace(&db, &workspace).map_err(ApiError::bad_request)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn settings(State(state): State<Arc<AppState>>) -> Result<Json<WorkspaceState>, ApiError> {
    let workspace = state.workspaces.lock().map_err(|_| ApiError::internal("Workspace lock poisoned"))?;
    Ok(Json(workspace.clone()))
}

async fn update_settings(State(state): State<Arc<AppState>>, Json(request): Json<WorkspacePatch>) -> Result<Json<WorkspaceState>, ApiError> {
    let db = state.db.lock().map_err(|_| ApiError::internal("Database lock poisoned"))?;
    let mut workspace = state.workspaces.lock().map_err(|_| ApiError::internal("Workspace lock poisoned"))?;
    if let Some(root) = request.general_root {
        let root = canonical_directory(&root).map_err(ApiError::bad_request)?;
        if workspace.projects.iter().any(|project| paths_overlap(&root, FsPath::new(&project.path))) {
            return Err(ApiError::bad_request(anyhow::anyhow!("General directory overlaps an existing project")));
        }
        workspace.general_root = root.display().to_string();
    }
    if let Some(sort) = request.project_sort {
        workspace.project_sort = sort;
    }
    if let Some(sort) = request.general_sort {
        workspace.general_sort = sort;
    }
    if let Some(sort) = request.other_sort {
        workspace.other_sort = sort;
    }
    if let Some(directory_sort) = request.directory_sort {
        if directory_sort.path.len() > 4096 {
            return Err(ApiError::bad_request(anyhow::anyhow!("Directory path is too long")));
        }
        workspace.directory_sort.insert(directory_sort.path, directory_sort.mode);
    }
    save_workspace(&db, &workspace).map_err(ApiError::bad_request)?;
    Ok(Json(workspace.clone()))
}

async fn reorder_workspace_items(State(state): State<Arc<AppState>>, Json(request): Json<ReorderRequest>) -> Result<Json<WorkspaceState>, ApiError> {
    if request.ids.len() > 2000 || request.scope.len() > 4096 {
        return Err(ApiError::bad_request(anyhow::anyhow!("Reorder request is too large")));
    }
    let mut seen = std::collections::HashSet::new();
    if request.ids.iter().any(|id| id.len() > 4096 || !seen.insert(id)) {
        return Err(ApiError::bad_request(anyhow::anyhow!("Reorder request contains invalid identifiers")));
    }

    let db = state.db.lock().map_err(|_| ApiError::internal("Database lock poisoned"))?;
    let mut workspace = state.workspaces.lock().map_err(|_| ApiError::internal("Workspace lock poisoned"))?;
    let positions = request.ids.iter().enumerate().map(|(index, id)| (id.as_str(), index as i64)).collect::<HashMap<_, _>>();
    match request.kind {
        ReorderKind::Projects => {
            for project in &mut workspace.projects {
                if let Some(position) = positions.get(project.id.as_str()) {
                    project.sort_order = *position;
                }
            }
            workspace.projects.sort_by_key(|project| project.sort_order);
            for (index, project) in workspace.projects.iter_mut().enumerate() {
                project.sort_order = index as i64;
            }
        }
        ReorderKind::Directories => {
            for directory in &mut workspace.other_directories {
                if let Some(position) = positions.get(directory.path.as_str()) {
                    directory.sort_order = *position;
                }
            }
            workspace.other_directories.sort_by_key(|directory| directory.sort_order);
            for (index, directory) in workspace.other_directories.iter_mut().enumerate() {
                directory.sort_order = index as i64;
            }
        }
        ReorderKind::Sessions => {
            workspace.session_order.insert(request.scope, request.ids);
        }
    }
    save_workspace(&db, &workspace).map_err(ApiError::bad_request)?;
    Ok(Json(workspace.clone()))
}

fn default_working_directory() -> PathBuf {
    dirs::home_dir().or_else(|| std::env::current_dir().ok()).unwrap_or_else(|| PathBuf::from("."))
}

async fn list_directories(Query(query): Query<DirectoryQuery>) -> Result<Json<DirectoryListing>, ApiError> {
    let requested = query.path.map(PathBuf::from).unwrap_or_else(default_working_directory);
    directory_listing(&requested, query.show_hidden).map(Json).map_err(ApiError::bad_request)
}

async fn create_directory(Json(request): Json<CreateDirectoryRequest>) -> Result<(StatusCode, Json<DirectoryEntry>), ApiError> {
    let parent = canonical_directory(&request.parent).map_err(ApiError::bad_request)?;
    let name = request.name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err(ApiError::bad_request(anyhow::anyhow!("Invalid directory name")));
    }
    let path = parent.join(name);
    std::fs::create_dir_all(&path).map_err(|error| ApiError::bad_request(anyhow::anyhow!("Cannot create directory: {error}")))?;
    Ok((
        StatusCode::CREATED,
        Json(DirectoryEntry {
            name: name.to_string(),
            path: path.display().to_string(),
        }),
    ))
}

fn directory_listing(requested: &std::path::Path, show_hidden: bool) -> anyhow::Result<DirectoryListing> {
    if requested.as_os_str().len() > 4096 {
        anyhow::bail!("Directory path is too long");
    }
    let path = requested
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("Cannot open directory '{}': {error}", requested.display()))?;
    if !path.is_dir() {
        anyhow::bail!("Path is not a directory: {}", path.display());
    }

    let mut entries = std::fs::read_dir(&path)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir() && (show_hidden || !entry.file_name().to_string_lossy().starts_with('.')))
        .map(|entry| DirectoryEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path().display().to_string(),
        })
        .collect::<Vec<_>>();
    entries.sort_by_cached_key(|entry| entry.name.to_lowercase());

    Ok(DirectoryListing {
        path: path.display().to_string(),
        parent: path.parent().map(|parent| parent.display().to_string()),
        home: dirs::home_dir().map(|home| home.display().to_string()),
        entries,
    })
}

async fn list_sessions(State(state): State<Arc<AppState>>) -> Json<Vec<SessionInfo>> {
    Json(state.manager.list())
}

async fn create_session(State(state): State<Arc<AppState>>, Json(request): Json<CreateSessionRequest>) -> Result<(StatusCode, Json<SessionInfo>), ApiError> {
    if !request.resume && request.resume_id.is_none() {
        let cwd = request
            .cwd
            .canonicalize()
            .map_err(|error| ApiError::bad_request(anyhow::anyhow!("Cannot open working directory: {error}")))?;
        let workspace = state.workspaces.lock().map_err(|_| ApiError::internal("Workspace lock poisoned"))?;
        let general = PathBuf::from(&workspace.general_root);
        let allowed = cwd.starts_with(&general) || workspace.projects.iter().any(|project| cwd.starts_with(FsPath::new(&project.path)));
        if !allowed {
            return Err(ApiError::bad_request(anyhow::anyhow!("New sessions must use the General directory or a Project directory")));
        }
    }
    let title = if request.title.trim().is_empty() {
        request
            .cwd
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(match request.agent {
                AgentKind::Claude => "Claude session",
                AgentKind::Codex => "Codex session",
            })
            .to_string()
    } else {
        request.title
    };
    let mut create = CreateSession::new(request.agent, title, request.cwd);
    create.rows = request.rows;
    create.cols = request.cols;
    if let Some(native_session_id) = request.resume_id {
        create.launch_mode = LaunchMode::Resume { native_session_id };
    } else if request.resume {
        create.launch_mode = LaunchMode::ResumePicker;
    }
    let session = state.manager.create(create).map_err(ApiError::bad_request)?;
    Ok((StatusCode::CREATED, Json(session)))
}

async fn session_details(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<SessionDetails>, ApiError> {
    let session = state
        .manager
        .get(&id)
        .map(|handle| handle.info())
        .ok_or_else(|| ApiError::not_found(anyhow::anyhow!("Managed session does not exist")))?;
    let app_type = match session.agent {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
    };
    let db = state.db.lock().map_err(|_| ApiError::internal("Database lock poisoned"))?;
    let record = session
        .native_session_id
        .as_deref()
        .map(|native_id| db.query_session(app_type, native_id))
        .transpose()
        .map_err(|error| ApiError::internal(error.to_string()))?
        .flatten();
    let usage = session
        .native_session_id
        .as_deref()
        .map(|native_id| db.query_session_usage_details(app_type, native_id))
        .transpose()
        .map_err(|error| ApiError::internal(error.to_string()))?
        .unwrap_or_default();

    let active_provider_key = if session.agent == AgentKind::Claude {
        "active_provider"
    } else {
        "active_codex_provider"
    };
    let provider_id = record
        .as_ref()
        .and_then(|item| item.profile_id.clone())
        .filter(|value| !value.is_empty())
        .or_else(|| db.get_setting(active_provider_key).filter(|value| !value.is_empty()));
    let provider_name = provider_id.as_deref().and_then(|provider_id| {
        db.get_providers(app_type)
            .ok()
            .and_then(|providers| providers.into_iter().find(|provider| provider.id == provider_id))
            .map(|provider| provider.name)
    });
    let profile_id = if session.agent == AgentKind::Claude {
        db.get_setting("active_profile").filter(|value| !value.is_empty())
    } else {
        None
    };
    let model = if usage.model.is_empty() {
        let key = if session.agent == AgentKind::Codex { "active_codex_model" } else { "" };
        (!key.is_empty()).then(|| db.get_setting(key)).flatten().filter(|value| !value.is_empty())
    } else {
        Some(usage.model)
    };
    let history_prompt = record.as_ref().map_or(0, |item| item.prompt_tokens);
    let history_completion = record.as_ref().map_or(0, |item| item.completion_tokens);

    Ok(Json(SessionDetails {
        managed_session_id: id,
        native_session_id: session.native_session_id,
        agent: session.agent,
        provider_id,
        provider_name,
        profile_id,
        model,
        prompt_tokens: usage.prompt_tokens.max(history_prompt),
        completion_tokens: usage.completion_tokens.max(history_completion),
        cache_read_tokens: usage.cache_read_tokens,
        cache_creation_tokens: usage.cache_creation_tokens,
        message_count: record.as_ref().map_or(0, |item| item.message_count),
    }))
}

async fn restart_session(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    state.manager.restart(&id).map_err(ApiError::not_found)?;
    Ok(StatusCode::ACCEPTED)
}

async fn close_session(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    state.manager.close(&id).map_err(ApiError::not_found)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn terminal_websocket(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let session = state
        .manager
        .get(&id)
        .ok_or_else(|| ApiError::not_found(anyhow::anyhow!("Managed session does not exist")))?;
    Ok(ws.on_upgrade(move |socket| terminal_socket(socket, session)))
}

async fn terminal_socket(socket: WebSocket, session: SessionHandle) {
    let (mut sender, mut receiver) = socket.split();
    let scrollback = session.scrollback();
    if !scrollback.is_empty() && sender.send(Message::Binary(scrollback)).await.is_err() {
        return;
    }
    if send_status(&mut sender, &session.info()).await.is_err() {
        return;
    }
    let mut events = session.subscribe();

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Binary(bytes))) => {
                        if session.write(bytes.to_vec()).is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        let Ok(control) = serde_json::from_str::<ClientControl>(&text) else {
                            continue;
                        };
                        match control {
                            ClientControl::Resize { rows, cols } => {
                                if session.resize(rows, cols).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(Message::Ping(bytes))) => {
                        if sender.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                }
            }
            event = events.recv() => {
                match event {
                    Ok(SessionStreamEvent::Output(bytes)) => {
                        if sender.send(Message::Binary(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Ok(SessionStreamEvent::Status(info)) => {
                        if send_status(&mut sender, &info).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast_error) => {
                        if matches!(broadcast_error, tokio::sync::broadcast::error::RecvError::Closed) {
                            break;
                        }
                        let reset = serde_json::json!({ "type": "reset" }).to_string();
                        if sender.send(Message::Text(reset)).await.is_err()
                            || sender.send(Message::Binary(session.scrollback())).await.is_err()
                        {
                            break;
                        }
                    }
                }
            }
        }
    }
}

async fn send_status(sender: &mut futures_util::stream::SplitSink<WebSocket, Message>, info: &SessionInfo) -> Result<(), axum::Error> {
    let message = serde_json::json!({ "type": "status", "session": info }).to_string();
    sender.send(Message::Text(message)).await
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(error: anyhow::Error) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
        }
    }

    fn not_found(error: anyhow::Error) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: error.to_string(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorResponse { error: self.message })).into_response()
    }
}

fn default_rows() -> u16 {
    24
}

fn default_cols() -> u16 {
    80
}

fn service_state_path() -> PathBuf {
    crate::core::config::data_dir().join("session-service.json")
}

fn write_service_state(address: SocketAddr) -> anyhow::Result<()> {
    let path = service_state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec_pretty(&serde_json::json!({
        "url": format!("http://{}", address),
        "pid": std::process::id(),
    }))?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    use std::io::Write;
    file.write_all(&body)?;
    file.sync_all()?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let terminate = async {
            if let Ok(mut signal) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                signal.recv().await;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::{directory_listing, is_local_authority, is_local_origin, router, workspace_response, Project, SortMode, WorkspaceDirectory, WorkspaceState};
    use crate::db::{sessions::SessionRecord, Db};
    use crate::session_runtime::SessionManager;
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use tower::ServiceExt;

    fn history_record(id: &str, path: &std::path::Path, file_mtime: &str, message_count: i64) -> SessionRecord {
        SessionRecord {
            id: id.into(),
            project_path: path.display().to_string(),
            profile_id: None,
            mode: "local".into(),
            start_time: file_mtime.into(),
            end_time: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            message_count,
            title: Some(id.into()),
            size_bytes: 1,
            file_mtime: file_mtime.into(),
            search_text: String::new(),
        }
    }

    #[test]
    fn accepts_only_loopback_browser_authorities() {
        for value in ["localhost:17321", "127.0.0.1:17321", "[::1]:17321"] {
            assert!(is_local_authority(value));
        }
        for value in ["example.com", "127.0.0.1.example.com", "0.0.0.0:17321"] {
            assert!(!is_local_authority(value));
        }
        assert!(is_local_origin("http://127.0.0.1:17321"));
        assert!(is_local_origin("tauri://localhost"));
        assert!(is_local_origin("http://tauri.localhost"));
        assert!(!is_local_origin("https://example.com"));
        assert!(!is_local_origin("https://tauri.localhost.example.com"));
    }

    #[test]
    fn directory_browser_lists_only_sorted_directories() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("zeta")).unwrap();
        std::fs::create_dir(root.path().join("Alpha")).unwrap();
        std::fs::write(root.path().join("notes.txt"), "ignored").unwrap();

        std::fs::create_dir(root.path().join(".hidden")).unwrap();

        let listing = directory_listing(root.path(), false).unwrap();
        let listing_with_hidden = directory_listing(root.path(), true).unwrap();

        assert_eq!(listing.entries.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(), ["Alpha", "zeta"]);
        assert_eq!(
            listing_with_hidden.entries.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(),
            [".hidden", "Alpha", "zeta"]
        );
        assert_eq!(listing.path, root.path().canonicalize().unwrap().display().to_string());
    }

    #[test]
    fn loads_workspace_state_saved_before_scoped_sorting() {
        let state: WorkspaceState = serde_json::from_value(serde_json::json!({
            "general_root": "/tmp/workbench",
            "projects": [],
            "other_directories": [{ "path": "/tmp/other", "pinned": false, "last_opened_ms": 1 }],
            "project_sort": "priority",
            "general_sort": "recent",
            "other_sort": "manual"
        }))
        .unwrap();

        assert_eq!(state.other_directories[0].sort_order, 0);
        assert!(state.directory_sort.is_empty());
        assert!(state.session_order.is_empty());
    }

    #[test]
    fn keeps_container_order_separate_from_scoped_session_order() {
        let root = tempfile::tempdir().unwrap();
        let general_root = root.path().join("general");
        let general_a = general_root.join("a");
        let general_b = general_root.join("b");
        let project_a = root.path().join("project-a");
        let project_b = root.path().join("project-b");
        for path in [&general_a, &general_b, &project_a, &project_b] {
            std::fs::create_dir_all(path).unwrap();
        }

        let db = Db::open(std::path::Path::new(":memory:")).unwrap();
        for (id, path, mtime, messages) in [
            ("project-a-old", project_a.as_path(), "2026-08-15 10:00:00", 1),
            ("project-a-new", project_a.as_path(), "2026-08-15 12:00:00", 9),
            ("project-b", project_b.as_path(), "2026-08-15 11:00:00", 3),
            ("general-a-old", general_a.as_path(), "2026-08-15 09:00:00", 1),
            ("general-a-new", general_a.as_path(), "2026-08-15 13:00:00", 2),
            ("general-b-old", general_b.as_path(), "2026-08-15 08:00:00", 1),
            ("general-b-new", general_b.as_path(), "2026-08-15 14:00:00", 2),
        ] {
            db.insert_session(&history_record(id, path, mtime, messages), "claude").unwrap();
        }

        let mut workspace = WorkspaceState {
            general_root: general_root.display().to_string(),
            projects: vec![
                Project {
                    id: "project-a".into(),
                    name: "Project A".into(),
                    path: project_a.display().to_string(),
                    pinned: true,
                    sort_order: 1,
                },
                Project {
                    id: "project-b".into(),
                    name: "Project B".into(),
                    path: project_b.display().to_string(),
                    pinned: false,
                    sort_order: 0,
                },
            ],
            other_directories: vec![
                WorkspaceDirectory {
                    path: general_a.display().to_string(),
                    pinned: false,
                    last_opened_ms: 1,
                    sort_order: 1,
                },
                WorkspaceDirectory {
                    path: general_b.display().to_string(),
                    pinned: false,
                    last_opened_ms: 2,
                    sort_order: 0,
                },
            ],
            project_sort: SortMode::Manual,
            general_sort: SortMode::Recent,
            other_sort: SortMode::Recent,
            directory_sort: std::collections::HashMap::from([(general_a.display().to_string(), SortMode::Manual)]),
            session_order: std::collections::HashMap::from([
                ("project:project-a".into(), vec!["claude:project-a-old".into(), "claude:project-a-new".into()]),
                (
                    format!("directory:{}", general_a.display()),
                    vec!["claude:general-a-old".into(), "claude:general-a-new".into()],
                ),
            ]),
        };

        let response = workspace_response(&db, &mut workspace, None).unwrap();

        assert_eq!(
            response.projects.iter().map(|group| group.project.id.as_str()).collect::<Vec<_>>(),
            ["project-b", "project-a"]
        );
        let project_a_history = &response.projects.iter().find(|group| group.project.id == "project-a").unwrap().history;
        assert_eq!(
            project_a_history.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            ["project-a-old", "project-a-new"]
        );
        assert_eq!(
            response.general.iter().map(|group| group.path.as_str()).collect::<Vec<_>>(),
            [general_b.to_str().unwrap(), general_a.to_str().unwrap()]
        );
        assert_eq!(
            response.general[0].items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            ["general-b-new", "general-b-old"]
        );
        assert_eq!(
            response.general[1].items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            ["general-a-old", "general-a-new"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn classifies_sessions_opened_through_a_workspace_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let project_alias = root.path().join("project-alias");
        let general = root.path().join("general");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(&general).unwrap();
        symlink(&project, &project_alias).unwrap();

        let db = Db::open(std::path::Path::new(":memory:")).unwrap();
        db.insert_session(
            &SessionRecord {
                id: "claude-through-symlink".into(),
                project_path: project_alias.display().to_string(),
                profile_id: None,
                mode: "local".into(),
                start_time: "2026-08-15 10:00:00".into(),
                end_time: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                message_count: 1,
                title: Some("Symlink session".into()),
                size_bytes: 1,
                file_mtime: "2026-08-15 10:00:00".into(),
                search_text: String::new(),
            },
            "claude",
        )
        .unwrap();
        let mut workspace = WorkspaceState {
            general_root: general.canonicalize().unwrap().display().to_string(),
            projects: vec![Project {
                id: "project-1".into(),
                name: "Project".into(),
                path: project.canonicalize().unwrap().display().to_string(),
                pinned: false,
                sort_order: 0,
            }],
            other_directories: Vec::new(),
            project_sort: SortMode::Priority,
            general_sort: SortMode::Recent,
            other_sort: SortMode::Recent,
            directory_sort: std::collections::HashMap::new(),
            session_order: std::collections::HashMap::new(),
        };

        let response = workspace_response(&db, &mut workspace, None).unwrap();

        assert_eq!(response.projects[0].history.len(), 1);
        assert_eq!(response.projects[0].history[0].id, "claude-through-symlink");
    }

    #[tokio::test]
    async fn serves_health_to_local_clients() {
        let response = router(SessionManager::new())
            .oneshot(Request::builder().uri("/api/health").header(header::HOST, "127.0.0.1:17321").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn session_details_returns_not_found_for_unknown_managed_session() {
        let response = router(SessionManager::new())
            .oneshot(
                Request::builder()
                    .uri("/api/sessions/missing/details")
                    .header(header::HOST, "127.0.0.1:17321")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rejects_foreign_host_and_origin_headers() {
        for (host, origin) in [("example.com", None), ("127.0.0.1:17321", Some("https://example.com"))] {
            let mut request = Request::builder().uri("/api/health").header(header::HOST, host);
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            let response = router(SessionManager::new()).oneshot(request.body(Body::empty()).unwrap()).await.unwrap();

            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }

    #[tokio::test]
    async fn allows_cors_only_for_trusted_local_frontends() {
        let response = router(SessionManager::new())
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/settings")
                    .header(header::HOST, "127.0.0.1:17321")
                    .header(header::ORIGIN, "tauri://localhost")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "PATCH")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(), "tauri://localhost");
        assert_eq!(response.headers().get(header::ACCESS_CONTROL_ALLOW_HEADERS).unwrap(), "Content-Type");
    }

    #[tokio::test]
    async fn rejects_invalid_session_creation() {
        let response = router(SessionManager::new())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sessions")
                    .header(header::HOST, "localhost:17321")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "agent": "codex",
                            "title": "",
                            "cwd": "/definitely/missing/akironmux-directory"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
