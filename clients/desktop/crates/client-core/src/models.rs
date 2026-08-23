use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub const LOCAL_PROFILE_ID: &str = "local";
pub const DEFAULT_LOCAL_ADDRESS: &str = "http://127.0.0.1:17321";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Starting,
    Running,
    Exited,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttentionKind {
    Input,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortMode {
    #[default]
    Priority,
    Recent,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendProfile {
    pub id: String,
    pub name: String,
    pub kind: BackendKind,
    pub address: String,
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub has_credential: bool,
    #[serde(default)]
    pub requires_auth: bool,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl BackendProfile {
    pub fn local() -> Self {
        Self {
            id: LOCAL_PROFILE_ID.into(),
            name: "Local".into(),
            kind: BackendKind::Local,
            address: DEFAULT_LOCAL_ADDRESS.into(),
            instance_id: None,
            has_credential: false,
            requires_auth: false,
            capabilities: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendProfileState {
    pub profiles: Vec<BackendProfile>,
    pub active_profile_id: String,
}

impl Default for BackendProfileState {
    fn default() -> Self {
        Self {
            profiles: vec![BackendProfile::local()],
            active_profile_id: LOCAL_PROFILE_ID.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendHealth {
    pub instance_id: String,
    pub api_protocol: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub agent: Agent,
    pub title: String,
    pub cwd: String,
    pub status: SessionStatus,
    pub created_at_ms: u64,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub native_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDetails {
    pub managed_session_id: String,
    pub native_session_id: Option<String>,
    pub agent: Agent,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub profile_id: Option<String>,
    pub model: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub message_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub id: String,
    pub agent: Agent,
    pub title: String,
    pub cwd: String,
    pub start_time: String,
    pub end_time: Option<String>,
    pub file_mtime: String,
    pub message_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub pinned: bool,
    pub sort_order: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryGroup {
    pub path: String,
    pub available: bool,
    pub items: Vec<HistoryItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceResponse {
    pub general_root: String,
    pub projects: Vec<ProjectHistory>,
    pub general: Vec<DirectoryGroup>,
    pub other: Vec<DirectoryGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectHistory {
    pub project: Project,
    pub history: Vec<HistoryItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtherDirectory {
    pub path: String,
    pub pinned: bool,
    pub last_opened_ms: u64,
    pub sort_order: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsResponse {
    pub general_root: String,
    pub projects: Vec<Project>,
    pub other_directories: Vec<OtherDirectory>,
    pub project_sort: SortMode,
    pub general_sort: SortMode,
    pub other_sort: SortMode,
    pub directory_sort: HashMap<String, SortMode>,
    pub session_order: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryListing {
    pub path: String,
    pub parent: Option<String>,
    pub home: Option<String>,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Locale {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloseBehavior {
    Tray,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClientPreferences {
    pub version: u32,
    pub locale: Locale,
    pub theme: ThemeMode,
    pub material_enabled: bool,
    pub material_transparency: u8,
    pub terminal_font_size: f32,
    pub sidebar_width: f32,
    pub close_behavior: CloseBehavior,
    pub workspace_icons: HashMap<String, String>,
    pub active_sessions: HashMap<String, String>,
    pub sidebar_expansion: HashMap<String, Vec<String>>,
}

impl Default for ClientPreferences {
    fn default() -> Self {
        Self {
            version: 1,
            locale: Locale::English,
            theme: ThemeMode::System,
            material_enabled: true,
            material_transparency: 30,
            terminal_font_size: 12.0,
            sidebar_width: 224.0,
            close_behavior: CloseBehavior::Tray,
            workspace_icons: HashMap::new(),
            active_sessions: HashMap::new(),
            sidebar_expansion: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateSessionRequest {
    pub agent: Agent,
    pub title: String,
    pub cwd: String,
    pub rows: u16,
    pub cols: u16,
    pub resume: bool,
    pub resume_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectRequest {
    pub id: Option<String>,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SortRequest {
    pub section: String,
    pub mode: SortMode,
    pub directory: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReorderRequest {
    pub section: String,
    pub ids: Vec<String>,
    pub directory: Option<String>,
}
