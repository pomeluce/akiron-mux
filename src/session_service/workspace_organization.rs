use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use serde::{Deserialize, Serialize};

use crate::{
    agent::AgentKind,
    db::{sessions::SessionRecord, Db},
};

const WORKSPACE_SETTING_KEY: &str = "akironmux.workspace";

#[derive(Debug)]
pub(crate) enum WorkspaceError {
    Invalid(anyhow::Error),
    NotFound(anyhow::Error),
    Unavailable(&'static str),
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(error) | Self::NotFound(error) => error.fmt(formatter),
            Self::Unavailable(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for WorkspaceError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Project {
    id: String,
    name: String,
    path: String,
    pinned: bool,
    sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WorkspaceDirectory {
    path: String,
    pinned: bool,
    last_opened_ms: i64,
    #[serde(default)]
    sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WorkspaceSettings {
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

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            general_root: default_general_root().display().to_string(),
            projects: Vec::new(),
            other_directories: Vec::new(),
            project_sort: SortMode::Priority,
            general_sort: SortMode::Recent,
            other_sort: SortMode::Recent,
            directory_sort: HashMap::new(),
            session_order: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SortMode {
    Priority,
    Recent,
    Manual,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct HistoryItem {
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
struct HistoryDirectory {
    path: String,
    available: bool,
    items: Vec<HistoryItem>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WorkspaceResponse {
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

#[derive(Default)]
pub(crate) struct ProjectChanges {
    pub(crate) name: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) pinned: Option<bool>,
}

#[derive(Default)]
pub(crate) struct WorkspaceChanges {
    pub(crate) general_root: Option<String>,
    pub(crate) project_sort: Option<SortMode>,
    pub(crate) general_sort: Option<SortMode>,
    pub(crate) other_sort: Option<SortMode>,
    pub(crate) directory_sort: Option<(String, SortMode)>,
}

pub(crate) struct WorkspaceOrganization {
    db: Arc<Mutex<Db>>,
    state: Mutex<WorkspaceSettings>,
}

impl WorkspaceOrganization {
    pub(crate) fn load(db: Arc<Mutex<Db>>) -> Result<Self, WorkspaceError> {
        let state = {
            let database = db.lock().map_err(|_| WorkspaceError::Unavailable("Database lock poisoned"))?;
            database
                .get_setting(WORKSPACE_SETTING_KEY)
                .and_then(|value| serde_json::from_str(&value).ok())
                .unwrap_or_default()
        };
        Ok(Self { db, state: Mutex::new(state) })
    }

    pub(crate) fn settings(&self) -> Result<WorkspaceSettings, WorkspaceError> {
        Ok(self.lock_state()?.clone())
    }

    pub(crate) fn view(&self, search: Option<&str>) -> Result<WorkspaceResponse, WorkspaceError> {
        let mut current = self.lock_state()?;
        let mut next = current.clone();
        let database = self.lock_db()?;
        let response = build_response(&database, &mut next, search).map_err(WorkspaceError::Invalid)?;
        persist(&database, &next)?;
        *current = next;
        Ok(response)
    }

    pub(crate) fn history(&self, search: Option<&str>) -> Result<Vec<HistoryItem>, WorkspaceError> {
        let _state = self.lock_state()?;
        let database = self.lock_db()?;
        let records = database.query_all_sessions(search, 2000).map_err(|error| WorkspaceError::Invalid(error.into()))?;
        Ok(records.into_iter().map(|(app_type, record)| history_item(app_type, record)).collect())
    }

    pub(crate) fn create_project(&self, path: &str, name: Option<String>) -> Result<Project, WorkspaceError> {
        let path = canonical_directory(path)?;
        self.mutate(|workspace| {
            let general = PathBuf::from(&workspace.general_root);
            if paths_overlap(&path, &general) || workspace.projects.iter().any(|project| paths_overlap(&path, Path::new(&project.path))) {
                return Err(invalid("Project directory overlaps an existing workspace"));
            }
            let project = Project {
                id: format!("project-{}", chrono::Utc::now().timestamp_millis()),
                name: name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| path.file_name().and_then(|name| name.to_str()).unwrap_or("Project").to_string()),
                path: path.display().to_string(),
                pinned: false,
                sort_order: workspace.projects.len() as i64,
            };
            workspace.projects.push(project.clone());
            Ok(project)
        })
    }

    pub(crate) fn update_project(&self, id: &str, changes: ProjectChanges) -> Result<Project, WorkspaceError> {
        self.mutate(|workspace| {
            let index = workspace
                .projects
                .iter()
                .position(|project| project.id == id)
                .ok_or_else(|| not_found("Project does not exist"))?;
            if let Some(path) = changes.path {
                let path = canonical_directory(&path)?;
                let general = PathBuf::from(&workspace.general_root);
                if paths_overlap(&path, &general)
                    || workspace
                        .projects
                        .iter()
                        .enumerate()
                        .any(|(other, project)| other != index && paths_overlap(&path, Path::new(&project.path)))
                {
                    return Err(invalid("Project directory overlaps an existing workspace"));
                }
                workspace.projects[index].path = path.display().to_string();
            }
            if let Some(name) = changes.name.filter(|name| !name.trim().is_empty()) {
                workspace.projects[index].name = name.trim().to_string();
            }
            if let Some(pinned) = changes.pinned {
                workspace.projects[index].pinned = pinned;
            }
            Ok(workspace.projects[index].clone())
        })
    }

    pub(crate) fn delete_project(&self, id: &str) -> Result<(), WorkspaceError> {
        self.mutate(|workspace| {
            let before = workspace.projects.len();
            workspace.projects.retain(|project| project.id != id);
            if workspace.projects.len() == before {
                return Err(not_found("Project does not exist"));
            }
            Ok(())
        })
    }

    pub(crate) fn update_settings(&self, changes: WorkspaceChanges) -> Result<WorkspaceSettings, WorkspaceError> {
        self.mutate(|workspace| {
            if let Some(root) = changes.general_root {
                let root = canonical_directory(&root)?;
                if workspace.projects.iter().any(|project| paths_overlap(&root, Path::new(&project.path))) {
                    return Err(invalid("General directory overlaps an existing project"));
                }
                workspace.general_root = root.display().to_string();
            }
            if let Some(sort) = changes.project_sort {
                workspace.project_sort = sort;
            }
            if let Some(sort) = changes.general_sort {
                workspace.general_sort = sort;
            }
            if let Some(sort) = changes.other_sort {
                workspace.other_sort = sort;
            }
            if let Some((path, mode)) = changes.directory_sort {
                if path.len() > 4096 {
                    return Err(invalid("Directory path is too long"));
                }
                workspace.directory_sort.insert(path, mode);
            }
            Ok(workspace.clone())
        })
    }

    pub(crate) fn reorder_projects(&self, ids: Vec<String>) -> Result<WorkspaceSettings, WorkspaceError> {
        validate_reorder(&ids, "")?;
        self.mutate(|workspace| {
            let positions = positions(&ids);
            for project in &mut workspace.projects {
                if let Some(position) = positions.get(project.id.as_str()) {
                    project.sort_order = *position;
                }
            }
            workspace.projects.sort_by_key(|project| project.sort_order);
            for (index, project) in workspace.projects.iter_mut().enumerate() {
                project.sort_order = index as i64;
            }
            Ok(workspace.clone())
        })
    }

    pub(crate) fn reorder_directories(&self, ids: Vec<String>) -> Result<WorkspaceSettings, WorkspaceError> {
        validate_reorder(&ids, "")?;
        self.mutate(|workspace| {
            let positions = positions(&ids);
            for directory in &mut workspace.other_directories {
                if let Some(position) = positions.get(directory.path.as_str()) {
                    directory.sort_order = *position;
                }
            }
            workspace.other_directories.sort_by_key(|directory| directory.sort_order);
            for (index, directory) in workspace.other_directories.iter_mut().enumerate() {
                directory.sort_order = index as i64;
            }
            Ok(workspace.clone())
        })
    }

    pub(crate) fn reorder_sessions(&self, scope: String, ids: Vec<String>) -> Result<WorkspaceSettings, WorkspaceError> {
        validate_reorder(&ids, &scope)?;
        self.mutate(|workspace| {
            workspace.session_order.insert(scope, ids);
            Ok(workspace.clone())
        })
    }

    pub(crate) fn allows_new_session(&self, cwd: &Path) -> Result<bool, WorkspaceError> {
        let workspace = self.lock_state()?;
        let general = PathBuf::from(&workspace.general_root);
        Ok(cwd.starts_with(&general) || workspace.projects.iter().any(|project| cwd.starts_with(Path::new(&project.path))))
    }

    fn mutate<T>(&self, change: impl FnOnce(&mut WorkspaceSettings) -> Result<T, WorkspaceError>) -> Result<T, WorkspaceError> {
        let mut current = self.lock_state()?;
        let mut next = current.clone();
        let result = change(&mut next)?;
        let database = self.lock_db()?;
        persist(&database, &next)?;
        *current = next;
        Ok(result)
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, WorkspaceSettings>, WorkspaceError> {
        self.state.lock().map_err(|_| WorkspaceError::Unavailable("Workspace lock poisoned"))
    }

    fn lock_db(&self) -> Result<MutexGuard<'_, Db>, WorkspaceError> {
        self.db.lock().map_err(|_| WorkspaceError::Unavailable("Database lock poisoned"))
    }
}

fn persist(database: &Db, workspace: &WorkspaceSettings) -> Result<(), WorkspaceError> {
    let serialized = serde_json::to_string(workspace).map_err(|error| WorkspaceError::Invalid(error.into()))?;
    database
        .set_setting(WORKSPACE_SETTING_KEY, &serialized)
        .map_err(|error| WorkspaceError::Invalid(error.into()))?;
    Ok(())
}

fn invalid(message: &str) -> WorkspaceError {
    WorkspaceError::Invalid(anyhow::anyhow!(message.to_owned()))
}

fn not_found(message: &str) -> WorkspaceError {
    WorkspaceError::NotFound(anyhow::anyhow!(message.to_owned()))
}

fn canonical_directory(path: &str) -> Result<PathBuf, WorkspaceError> {
    let path = PathBuf::from(path);
    if path.as_os_str().len() > 4096 {
        return Err(invalid("Directory path is too long"));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| WorkspaceError::Invalid(anyhow::anyhow!("Cannot open directory '{}': {error}", path.display())))?;
    if !canonical.is_dir() {
        return Err(invalid(&format!("Path is not a directory: {}", canonical.display())));
    }
    Ok(canonical)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn canonicalize_for_comparison(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn build_response(database: &Db, workspace: &mut WorkspaceSettings, search: Option<&str>) -> anyhow::Result<WorkspaceResponse> {
    let records = database.query_all_sessions(search, 2000)?;
    let mut projects = workspace
        .projects
        .iter()
        .cloned()
        .map(|project| ProjectGroup { project, history: Vec::new() })
        .collect::<Vec<_>>();
    let project_roots = projects.iter().map(|group| canonicalize_for_comparison(Path::new(&group.project.path))).collect::<Vec<_>>();
    let mut general: HashMap<String, HistoryDirectory> = HashMap::new();
    let mut other: HashMap<String, HistoryDirectory> = HashMap::new();
    let general_root = canonicalize_for_comparison(Path::new(&workspace.general_root));

    for (app_type, record) in records {
        if record.project_path.trim().is_empty() {
            continue;
        }
        let cwd = PathBuf::from(&record.project_path);
        let item = history_item(app_type, record);
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

fn validate_reorder(ids: &[String], scope: &str) -> Result<(), WorkspaceError> {
    if ids.len() > 2000 || scope.len() > 4096 {
        return Err(invalid("Reorder request is too large"));
    }
    let mut seen = HashSet::new();
    if ids.iter().any(|id| id.len() > 4096 || !seen.insert(id)) {
        return Err(invalid("Reorder request contains invalid identifiers"));
    }
    Ok(())
}

fn positions(ids: &[String]) -> HashMap<&str, i64> {
    ids.iter().enumerate().map(|(index, id)| (id.as_str(), index as i64)).collect()
}

fn default_general_root() -> PathBuf {
    let home = dirs::home_dir().or_else(|| std::env::current_dir().ok()).unwrap_or_else(|| PathBuf::from("."));
    let workbench = home.join("workbench");
    if workbench.is_dir() {
        workbench
    } else {
        home
    }
}

#[cfg(test)]
mod tests {
    use super::{Project, ProjectChanges, SortMode, WorkspaceChanges, WorkspaceError, WorkspaceOrganization, WorkspaceSettings, WORKSPACE_SETTING_KEY};
    use crate::db::{sessions::SessionRecord, Db};
    use std::sync::{Arc, Mutex};

    fn history_record(id: &str, path: &std::path::Path, file_mtime: &str, message_count: i64) -> SessionRecord {
        SessionRecord {
            id: id.into(),
            project_path: path.display().to_string(),
            profile_id: None,
            parent_thread_id: None,
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

    fn load_with_state(database: Db, state: &WorkspaceSettings) -> (Arc<Mutex<Db>>, WorkspaceOrganization) {
        database.set_setting(WORKSPACE_SETTING_KEY, &serde_json::to_string(state).unwrap()).unwrap();
        let database = Arc::new(Mutex::new(database));
        let organization = WorkspaceOrganization::load(Arc::clone(&database)).unwrap();
        (database, organization)
    }

    #[test]
    fn loads_workspace_state_saved_before_scoped_sorting() {
        let database = Db::open(std::path::Path::new(":memory:")).unwrap();
        database
            .set_setting(
                WORKSPACE_SETTING_KEY,
                &serde_json::json!({
                    "general_root": "/tmp/workbench",
                    "projects": [],
                    "other_directories": [{ "path": "/tmp/other", "pinned": false, "last_opened_ms": 1 }],
                    "project_sort": "priority",
                    "general_sort": "recent",
                    "other_sort": "manual"
                })
                .to_string(),
            )
            .unwrap();
        let organization = WorkspaceOrganization::load(Arc::new(Mutex::new(database))).unwrap();

        let settings = organization.settings().unwrap();
        assert_eq!(settings.other_directories[0].sort_order, 0);
        assert!(settings.directory_sort.is_empty());
        assert!(settings.session_order.is_empty());
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

        let database = Db::open(std::path::Path::new(":memory:")).unwrap();
        for (id, path, mtime, messages) in [
            ("project-a-old", project_a.as_path(), "2026-08-15 10:00:00", 1),
            ("project-a-new", project_a.as_path(), "2026-08-15 12:00:00", 9),
            ("project-b", project_b.as_path(), "2026-08-15 11:00:00", 3),
            ("general-a-old", general_a.as_path(), "2026-08-15 09:00:00", 1),
            ("general-a-new", general_a.as_path(), "2026-08-15 13:00:00", 2),
            ("general-b-old", general_b.as_path(), "2026-08-15 08:00:00", 1),
            ("general-b-new", general_b.as_path(), "2026-08-15 14:00:00", 2),
        ] {
            database.insert_session(&history_record(id, path, mtime, messages), "claude").unwrap();
        }
        let state: WorkspaceSettings = serde_json::from_value(serde_json::json!({
            "general_root": general_root,
            "projects": [
                { "id": "project-a", "name": "Project A", "path": project_a, "pinned": true, "sort_order": 1 },
                { "id": "project-b", "name": "Project B", "path": project_b, "pinned": false, "sort_order": 0 }
            ],
            "other_directories": [
                { "path": general_a, "pinned": false, "last_opened_ms": 1, "sort_order": 1 },
                { "path": general_b, "pinned": false, "last_opened_ms": 2, "sort_order": 0 }
            ],
            "project_sort": "manual",
            "general_sort": "recent",
            "other_sort": "recent",
            "directory_sort": { general_a.display().to_string(): "manual" },
            "session_order": {
                "project:project-a": ["claude:project-a-old", "claude:project-a-new"],
                format!("directory:{}", general_a.display()): ["claude:general-a-old", "claude:general-a-new"]
            }
        }))
        .unwrap();
        let (_, organization) = load_with_state(database, &state);

        let response = organization.view(None).unwrap();

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

    #[test]
    fn failed_persistence_does_not_publish_a_project_mutation() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let database = Db::open(std::path::Path::new(":memory:")).unwrap();
        let database = Arc::new(Mutex::new(database));
        let organization = WorkspaceOrganization::load(Arc::clone(&database)).unwrap();
        database.lock().unwrap().conn().execute("DROP TABLE settings", []).unwrap();

        assert!(matches!(organization.create_project(project.to_str().unwrap(), None), Err(WorkspaceError::Invalid(_))));
        assert!(organization.settings().unwrap().projects.is_empty());
    }

    #[test]
    fn project_and_general_changes_share_validation_and_persistence() {
        let root = tempfile::tempdir().unwrap();
        let general = root.path().join("general");
        let project = root.path().join("project");
        std::fs::create_dir(&general).unwrap();
        std::fs::create_dir(&project).unwrap();
        let database = Arc::new(Mutex::new(Db::open(std::path::Path::new(":memory:")).unwrap()));
        let organization = WorkspaceOrganization::load(Arc::clone(&database)).unwrap();

        organization
            .update_settings(WorkspaceChanges {
                general_root: Some(general.display().to_string()),
                ..WorkspaceChanges::default()
            })
            .unwrap();
        let created = organization.create_project(project.to_str().unwrap(), Some(" Original ".into())).unwrap();
        let updated = organization
            .update_project(
                &created.id,
                ProjectChanges {
                    name: Some("Renamed".into()),
                    pinned: Some(true),
                    ..ProjectChanges::default()
                },
            )
            .unwrap();

        assert_eq!(updated.name, "Renamed");
        assert!(updated.pinned);
        assert!(matches!(
            organization.update_settings(WorkspaceChanges {
                general_root: Some(project.display().to_string()),
                ..WorkspaceChanges::default()
            }),
            Err(WorkspaceError::Invalid(_))
        ));
        let reloaded = WorkspaceOrganization::load(database).unwrap().settings().unwrap();
        assert_eq!(reloaded.general_root, general.canonicalize().unwrap().display().to_string());
        assert_eq!(reloaded.projects, vec![updated]);
    }

    #[test]
    fn gui_workspace_history_hides_codex_children_and_aggregates_messages() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let general = root.path().join("general");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(&general).unwrap();
        let database = Db::open(std::path::Path::new(":memory:")).unwrap();
        let mut parent = history_record("parent-session", &project, "2026-08-15 10:00:00", 2);
        parent.mode = "direct".into();
        let mut child = history_record("child-session", &project, "2026-08-15 10:01:00", 3);
        child.mode = "direct".into();
        child.parent_thread_id = Some(parent.id.clone());
        database.insert_session(&parent, "codex").unwrap();
        database.insert_session(&child, "codex").unwrap();
        let state = WorkspaceSettings {
            general_root: general.display().to_string(),
            projects: vec![Project {
                id: "project".into(),
                name: "Project".into(),
                path: project.display().to_string(),
                pinned: false,
                sort_order: 0,
            }],
            project_sort: SortMode::Recent,
            ..WorkspaceSettings::default()
        };
        let (_, organization) = load_with_state(database, &state);

        let response = organization.view(None).unwrap();
        assert_eq!(response.projects[0].history.len(), 1);
        assert_eq!(response.projects[0].history[0].id, "parent-session");
        assert_eq!(response.projects[0].history[0].message_count, 5);
    }

    #[cfg(unix)]
    #[test]
    fn classifies_sessions_opened_through_a_project_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let project_alias = root.path().join("project-alias");
        let general = root.path().join("general");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(&general).unwrap();
        symlink(&project, &project_alias).unwrap();
        let database = Db::open(std::path::Path::new(":memory:")).unwrap();
        database
            .insert_session(&history_record("claude-through-symlink", &project_alias, "2026-08-15 10:00:00", 1), "claude")
            .unwrap();
        let state = WorkspaceSettings {
            general_root: general.canonicalize().unwrap().display().to_string(),
            projects: vec![Project {
                id: "project-1".into(),
                name: "Project".into(),
                path: project.canonicalize().unwrap().display().to_string(),
                pinned: false,
                sort_order: 0,
            }],
            ..WorkspaceSettings::default()
        };
        let (_, organization) = load_with_state(database, &state);

        let response = organization.view(None).unwrap();
        assert_eq!(response.projects[0].history[0].id, "claude-through-symlink");
    }
}
