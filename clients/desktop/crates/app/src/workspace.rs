use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use akmux_client_core::{
    Agent, BackendApi, BackendKind, BackendLifecycleOutcome, BackendProfile, BackendProfileIntent, BackendProfileLifecycle, BackendProfileState, ClientPaths, ClientPreferences,
    ClientStateStore, CloseBehavior, CreateSessionRequest, CredentialStore, DirectoryListing, HistoryItem, Locale, MemoryRecoveryCredentialStore, Project, SessionDetails,
    SessionInfo, SettingsResponse, SortMode, TerminalTransportConfig, ThemeMode as PreferenceThemeMode, WorkspaceResponse, operation_id,
};
use akmux_desktop_platform::{DesktopTray, NativeCredentialStore, SingleInstance, TrayAction};
use akmux_terminal::{TerminalNotice, TerminalView};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable, StyledExt, Theme as UITheme, ThemeMode as UIThemeMode, TitleBar, WindowExt,
    button::{Button, ButtonVariants},
    dialog::{DialogClose, DialogFooter},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    sidebar::{Sidebar, SidebarCollapsible, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    v_flex,
};
use zeroize::Zeroizing;

struct DashboardSnapshot {
    sessions: Vec<SessionInfo>,
    workspaces: WorkspaceResponse,
    settings: SettingsResponse,
}

#[derive(Clone)]
enum NewSessionMode {
    General,
    Project,
}

#[derive(Clone)]
enum DirectoryPickerTarget {
    NewSession(NewSessionMode),
    Project(Option<Project>),
    GeneralRoot,
}

struct DirectoryPickerState {
    target: DirectoryPickerTarget,
    listing: Option<DirectoryListing>,
    show_hidden: bool,
    loading: bool,
    error: Option<String>,
    request_generation: u64,
}

pub struct DesktopWorkspace {
    runtime: Arc<tokio::runtime::Runtime>,
    window_handle: AnyWindowHandle,
    tray: DesktopTray,
    close_to_tray: Arc<AtomicBool>,
    _instance: SingleInstance,
    api: BackendApi,
    state_store: Arc<ClientStateStore>,
    credentials: Arc<NativeCredentialStore>,
    lifecycle: Arc<BackendProfileLifecycle>,
    recovery: Arc<MemoryRecoveryCredentialStore>,
    profiles: BackendProfileState,
    preferences: ClientPreferences,
    sessions: Vec<SessionInfo>,
    workspaces: Option<WorkspaceResponse>,
    settings: Option<SettingsResponse>,
    active_session_id: Option<String>,
    terminals: HashMap<String, Entity<TerminalView>>,
    attention: HashMap<String, akmux_client_core::AttentionKind>,
    recent_attention: HashMap<String, Instant>,
    sidebar_collapsed: bool,
    status_message: String,
    new_session_directory: Entity<InputState>,
    new_session_subdirectory: Entity<InputState>,
    directory_path: Entity<InputState>,
    backend_name: Entity<InputState>,
    backend_address: Entity<InputState>,
    backend_pairing_link: Entity<InputState>,
    project_name: Entity<InputState>,
    project_path: Entity<InputState>,
    general_root: Entity<InputState>,
    search_input: Entity<InputState>,
    search_query: String,
    directory_picker: Option<DirectoryPickerState>,
    pending_identity: Option<(String, String)>,
    backend_editing_profile: Option<BackendProfile>,
    generation: u64,
    _poll_task: Task<()>,
    _tray_task: Task<()>,
    _activation_task: Task<()>,
    _native_pump_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl DesktopWorkspace {
    pub fn new(
        window: &mut Window,
        tray: DesktopTray,
        tray_events: async_channel::Receiver<TrayAction>,
        instance: SingleInstance,
        close_to_tray: Arc<AtomicBool>,
        cx: &mut Context<Self>,
    ) -> Self {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("akmux-desktop-io")
                .build()
                .expect("failed to create desktop I/O runtime"),
        );
        let api = BackendApi::new().expect("failed to create backend HTTP client");
        let state_store = Arc::new(ClientStateStore::new(ClientPaths::new(config_directory())));
        let credentials = Arc::new(NativeCredentialStore);
        let lifecycle = Arc::new(BackendProfileLifecycle::new(state_store.clone(), credentials.clone(), api.clone()));
        let profiles = lifecycle.state().unwrap_or_else(|_| state_store.load_backend_profiles().unwrap_or_default());
        let preferences = state_store.load_preferences().unwrap_or_default();
        let chinese = matches!(preferences.locale, Locale::SimplifiedChinese);
        close_to_tray.store(matches!(preferences.close_behavior, CloseBehavior::Tray), Ordering::Relaxed);
        let activations = instance.activations();
        let new_session_directory = cx.new(|cx| InputState::new(window, cx).placeholder(localized(chinese, "Working directory", "工作目录")));
        let new_session_subdirectory = cx.new(|cx| InputState::new(window, cx).placeholder(localized(chinese, "Optional isolated subdirectory", "可选隔离子目录")));
        let directory_path = cx.new(|cx| InputState::new(window, cx).placeholder(localized(chinese, "Directory path", "目录路径")));
        let backend_name = cx.new(|cx| InputState::new(window, cx).placeholder(localized(chinese, "Backend name", "后端名称")));
        let backend_address = cx.new(|cx| InputState::new(window, cx).placeholder("https://backend.example.com"));
        let backend_pairing_link = cx.new(|cx| InputState::new(window, cx).placeholder("akmux://pair?url=…&code=…"));
        let project_name = cx.new(|cx| InputState::new(window, cx).placeholder(localized(chinese, "Project name", "项目名称")));
        let project_path = cx.new(|cx| InputState::new(window, cx).placeholder(localized(chinese, "Project directory", "项目目录")));
        let general_root = cx.new(|cx| InputState::new(window, cx).placeholder(localized(chinese, "General workspace root", "通用工作区根目录")));
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder(localized(chinese, "Search title, directory, or agent", "搜索标题、目录或工具")));
        let mut _subscriptions = vec![cx.subscribe_in(&search_input, window, {
            let search_input = search_input.clone();
            move |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.search_query = search_input.read(cx).value().to_string();
                    cx.notify();
                }
            }
        })];
        _subscriptions.push(cx.observe_window_appearance(window, |this, window, cx| {
            if matches!(this.preferences.theme, PreferenceThemeMode::System) {
                UITheme::change(window.appearance(), Some(window), cx);
            }
            this.apply_material(window);
            cx.notify();
        }));
        let theme = match preferences.theme {
            PreferenceThemeMode::Light => UIThemeMode::Light,
            PreferenceThemeMode::Dark => UIThemeMode::Dark,
            PreferenceThemeMode::System => window.appearance().into(),
        };
        UITheme::change(theme, Some(window), cx);
        apply_window_material(window, preferences.material_enabled);
        let mut this = Self {
            runtime,
            window_handle: window.window_handle(),
            tray,
            close_to_tray,
            _instance: instance,
            api,
            state_store,
            credentials,
            lifecycle,
            recovery: Arc::new(MemoryRecoveryCredentialStore::default()),
            profiles,
            preferences,
            sessions: Vec::new(),
            workspaces: None,
            settings: None,
            active_session_id: None,
            terminals: HashMap::new(),
            attention: HashMap::new(),
            recent_attention: HashMap::new(),
            sidebar_collapsed: false,
            status_message: localized(chinese, "Connecting to backend…", "正在连接后端…").into(),
            new_session_directory,
            new_session_subdirectory,
            directory_path,
            backend_name,
            backend_address,
            backend_pairing_link,
            project_name,
            project_path,
            general_root,
            search_input,
            search_query: String::new(),
            directory_picker: None,
            pending_identity: None,
            backend_editing_profile: None,
            generation: 0,
            _poll_task: Task::ready(()),
            _tray_task: Task::ready(()),
            _activation_task: Task::ready(()),
            _native_pump_task: Task::ready(()),
            _subscriptions,
        };
        this.start_native_tasks(tray_events, activations, cx);
        this.start_polling(cx);
        this
    }

    fn start_native_tasks(&mut self, tray_events: async_channel::Receiver<TrayAction>, activations: async_channel::Receiver<()>, cx: &mut Context<Self>) {
        self._tray_task = cx.spawn(async move |this, cx| {
            while let Ok(action) = tray_events.recv().await {
                if this
                    .update(cx, |this, cx| match action {
                        TrayAction::Activate => this.activate_window(cx),
                        TrayAction::SelectSession(session_id) => {
                            this.select_session(session_id, cx);
                            this.activate_window(cx);
                        }
                        TrayAction::Quit => cx.quit(),
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self._activation_task = cx.spawn(async move |this, cx| {
            while activations.recv().await.is_ok() {
                if this.update(cx, |this, cx| this.activate_window(cx)).is_err() {
                    break;
                }
            }
        });
        let executor = cx.background_executor().clone();
        self._native_pump_task = cx.spawn(async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(100)).await;
                if this.update(cx, |this, _| this.tray.pump_native_events()).is_err() {
                    break;
                }
            }
        });
    }

    fn activate_window(&self, cx: &mut Context<Self>) {
        cx.activate(true);
        let _ = self.window_handle.update(cx, |_, window, _| window.activate_window());
    }

    fn active_profile(&self) -> BackendProfile {
        self.profiles
            .profiles
            .iter()
            .find(|profile| profile.id == self.profiles.active_profile_id)
            .cloned()
            .unwrap_or_else(BackendProfile::local)
    }

    fn credential(&self, profile: &BackendProfile) -> Option<Arc<Zeroizing<String>>> {
        (profile.kind == BackendKind::Remote)
            .then(|| self.credentials.load(&profile.id).ok().map(Arc::new))
            .flatten()
    }

    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            loop {
                let credential = credential.as_deref().map(|value| value.as_str());
                let (sessions, workspaces, settings) = tokio::join!(
                    api.sessions(&profile, credential),
                    api.workspaces(&profile, credential, None),
                    api.settings(&profile, credential),
                );
                let result = match (sessions, workspaces, settings) {
                    (Ok(sessions), Ok(workspaces), Ok(settings)) => Ok(DashboardSnapshot { sessions, workspaces, settings }),
                    (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => Err(error.to_string()),
                };
                if tx.send(result).await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
        self._poll_task = cx.spawn(async move |this, cx| {
            while let Ok(result) = rx.recv().await {
                if this
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return;
                        }
                        match result {
                            Ok(snapshot) => {
                                this.sessions = snapshot
                                    .sessions
                                    .into_iter()
                                    .filter(|session| session.status != akmux_client_core::SessionStatus::Exited)
                                    .collect();
                                let tray_sessions = this.sessions.iter().map(|session| (session.id.clone(), session.title.clone())).collect::<Vec<_>>();
                                if let Err(error) = this.tray.update_sessions(&tray_sessions, matches!(this.preferences.locale, Locale::SimplifiedChinese)) {
                                    this.status_message = error.to_string();
                                }
                                this.terminals.retain(|session_id, _| this.sessions.iter().any(|session| &session.id == session_id));
                                this.workspaces = Some(snapshot.workspaces);
                                this.settings = Some(snapshot.settings);
                                this.status_message = localized(matches!(this.preferences.locale, Locale::SimplifiedChinese), "Connected", "已连接").into();
                                this.restore_or_select_session(cx);
                            }
                            Err(error) => this.status_message = error,
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
    }

    fn restore_or_select_session(&mut self, cx: &mut Context<Self>) {
        if self.active_session_id.as_ref().is_some_and(|id| self.sessions.iter().any(|session| &session.id == id)) {
            return;
        }
        let profile_id = self.active_profile().id;
        let preferred = self
            .preferences
            .active_sessions
            .get(&profile_id)
            .filter(|id| self.sessions.iter().any(|session| &session.id == *id))
            .cloned();
        let fallback = self
            .sessions
            .iter()
            .find(|session| matches!(session.status, akmux_client_core::SessionStatus::Running | akmux_client_core::SessionStatus::Starting))
            .map(|session| session.id.clone());
        if let Some(session_id) = preferred.or(fallback) {
            self.select_session(session_id, cx);
        } else {
            self.active_session_id = None;
            self.terminals.clear();
        }
    }

    fn select_session(&mut self, session_id: String, cx: &mut Context<Self>) {
        if self.active_session_id.as_deref() == Some(&session_id) && self.terminals.contains_key(&session_id) {
            return;
        }
        let profile = self.active_profile();
        self.attention.remove(&session_id);
        self.preferences.active_sessions.insert(profile.id.clone(), session_id.clone());
        let _ = self.state_store.save_preferences(&self.preferences);
        if !self.terminals.contains_key(&session_id) {
            let config = TerminalTransportConfig {
                api: self.api.clone(),
                profile: profile.clone(),
                credential: self.credential(&profile),
                session_id: session_id.clone(),
                recovery_store: self.recovery.clone(),
            };
            let handle = self.runtime.handle().clone();
            let terminal = cx.new(|cx| TerminalView::new(config, &handle, cx));
            terminal.update(cx, |terminal, cx| {
                terminal.set_font_size(self.preferences.terminal_font_size, cx);
                terminal.set_simplified_chinese(matches!(self.preferences.locale, Locale::SimplifiedChinese), cx);
            });
            self._subscriptions.push(cx.subscribe(&terminal, {
                let session_id = session_id.clone();
                move |this, _, notice, cx| this.handle_terminal_notice(&session_id, notice, cx)
            }));
            self.terminals.insert(session_id.clone(), terminal);
        }
        self.active_session_id = Some(session_id);
        if let Some(terminal) = self.active_session_id.as_ref().and_then(|id| self.terminals.get(id)) {
            let focus = terminal.read(cx).focus_handle();
            let _ = self.window_handle.update(cx, |_, window, cx| focus.focus(window, cx));
        }
        cx.notify();
    }

    fn on_workspace_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !event.keystroke.modifiers.control || !event.keystroke.key.eq_ignore_ascii_case("tab") || self.sessions.is_empty() {
            return;
        }
        let current = self
            .active_session_id
            .as_ref()
            .and_then(|id| self.sessions.iter().position(|session| &session.id == id))
            .unwrap_or(0);
        let next = if event.keystroke.modifiers.shift {
            current.checked_sub(1).unwrap_or(self.sessions.len() - 1)
        } else {
            (current + 1) % self.sessions.len()
        };
        self.select_session(self.sessions[next].id.clone(), cx);
        cx.stop_propagation();
    }

    fn handle_terminal_notice(&mut self, session_id: &str, notice: &TerminalNotice, cx: &mut Context<Self>) {
        match notice {
            TerminalNotice::Status(updated) => {
                if let Some(session) = self.sessions.iter_mut().find(|session| session.id == updated.id) {
                    *session = updated.clone();
                }
                if updated.status == akmux_client_core::SessionStatus::Exited {
                    let session_id = updated.id.clone();
                    let executor = cx.background_executor().clone();
                    cx.spawn(async move |this, cx| {
                        executor.timer(Duration::from_millis(650)).await;
                        let _ = this.update(cx, |this, cx| {
                            if this
                                .sessions
                                .iter()
                                .any(|session| session.id == session_id && session.status == akmux_client_core::SessionStatus::Exited)
                            {
                                this.sessions.retain(|session| session.id != session_id);
                                this.terminals.remove(&session_id);
                                this.restore_or_select_session(cx);
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
            }
            TerminalNotice::Attention(kind) => {
                if self.active_session_id.as_deref() == Some(session_id) {
                    return;
                }
                let dedup_key = format!("{session_id}:{kind:?}");
                let now = Instant::now();
                if self
                    .recent_attention
                    .get(&dedup_key)
                    .is_some_and(|previous| now.duration_since(*previous) < Duration::from_secs(2))
                {
                    return;
                }
                self.recent_attention.insert(dedup_key, now);
                self.attention.insert(session_id.to_owned(), *kind);
                if let Some(session) = self.sessions.iter().find(|session| session.id == session_id) {
                    let (title, body) = match (kind, &self.preferences.locale) {
                        (akmux_client_core::AttentionKind::Input, Locale::SimplifiedChinese) => ("会话等待操作", format!("{} · {}", agent_name(session.agent), session.title)),
                        (akmux_client_core::AttentionKind::Completed, Locale::SimplifiedChinese) => ("回答已完成", format!("{} · {}", agent_name(session.agent), session.title)),
                        (akmux_client_core::AttentionKind::Input, _) => ("Session needs attention", format!("{} · {}", agent_name(session.agent), session.title)),
                        (akmux_client_core::AttentionKind::Completed, _) => ("Response completed", format!("{} · {}", agent_name(session.agent), session.title)),
                    };
                    cx.show_system_notification(SystemNotification {
                        tag: format!("akmux-{session_id}").into(),
                        title: title.into(),
                        body: body.into(),
                        actions: Vec::new(),
                    });
                    let _ = self.window_handle.update(cx, |_, window, _| window.request_attention());
                }
            }
            TerminalNotice::AuthorizationRevoked => {
                self.status_message = localized(
                    matches!(self.preferences.locale, Locale::SimplifiedChinese),
                    "Backend authorization was revoked",
                    "后端授权已撤销",
                )
                .into();
            }
            TerminalNotice::OpenLink(url) => cx.open_url(url),
        }
        cx.notify();
    }

    fn toggle_sidebar(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn open_new_session(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let initial = self
            .settings
            .as_ref()
            .map(|settings| settings.general_root.as_str())
            .or_else(|| self.workspaces.as_ref().map(|workspaces| workspaces.general_root.as_str()))
            .unwrap_or_default();
        self.new_session_directory.update(cx, |input, cx| input.set_value(initial, window, cx));
        self.new_session_subdirectory.update(cx, |input, cx| input.set_value("", window, cx));
        self.show_new_session_dialog(NewSessionMode::General, window, cx);
    }

    fn open_project_session(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.new_session_directory.update(cx, |input, cx| input.set_value(path, window, cx));
        self.new_session_subdirectory.update(cx, |input, cx| input.set_value("", window, cx));
        self.show_new_session_dialog(NewSessionMode::Project, window, cx);
    }

    fn show_new_session_dialog(&mut self, mode: NewSessionMode, window: &mut Window, cx: &mut Context<Self>) {
        let directory = self.new_session_directory.clone();
        let subdirectory = self.new_session_subdirectory.clone();
        let is_general = matches!(mode, NewSessionMode::General);
        let chinese = matches!(self.preferences.locale, Locale::SimplifiedChinese);
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let codex_view = view.clone();
            let codex_directory = directory.clone();
            let codex_subdirectory = subdirectory.clone();
            let claude_view = view.clone();
            let claude_directory = directory.clone();
            let claude_subdirectory = subdirectory.clone();
            let browse_view = view.clone();
            let browse_directory = directory.clone();
            let browse_mode = mode.clone();
            dialog
                .title(if is_general {
                    localized(chinese, "New session", "新建会话")
                } else {
                    localized(chinese, "New project session", "新建项目会话")
                })
                .w(px(540.))
                .child(
                    v_flex()
                        .gap_3()
                        .child(div().text_sm().child(localized(chinese, "Working directory", "工作目录")))
                        .child(
                            h_flex().gap_2().child(Input::new(&directory).w_full()).child(
                                Button::new("browse-session-directory")
                                    .outline()
                                    .icon(IconName::Folder)
                                    .label(localized(chinese, "Browse", "浏览"))
                                    .on_click(move |_, window, cx| {
                                        let initial = browse_directory.read(cx).value().to_string();
                                        window.close_dialog(cx);
                                        browse_view.update(cx, |this, cx| {
                                            this.open_directory_picker(DirectoryPickerTarget::NewSession(browse_mode.clone()), initial, window, cx)
                                        });
                                    }),
                            ),
                        )
                        .when(is_general, |this| {
                            this.child(div().text_sm().child(localized(chinese, "Optional isolated subdirectory", "可选隔离子目录")))
                                .child(Input::new(&subdirectory).w_full())
                                .child(div().text_xs().child(localized(
                                    chinese,
                                    "When set, the daemon creates this child directory before starting the session.",
                                    "填写后，守护进程会在启动会话前创建此子目录。",
                                )))
                        })
                        .child(div().text_xs().child(localized(
                            chinese,
                            "The daemon validates that the directory belongs to General or a Project.",
                            "守护进程会验证目录属于通用目录或某个项目。",
                        ))),
                )
                .footer(
                    DialogFooter::new()
                        .gap_2()
                        .child(DialogClose::new().child(Button::new("cancel-new-session").outline().label(localized(chinese, "Cancel", "取消"))))
                        .child(
                            Button::new("create-claude-session")
                                .outline()
                                .icon(IconName::Bot)
                                .label("Claude Code")
                                .on_click(move |_, window, cx| {
                                    let cwd = claude_directory.read(cx).value().to_string();
                                    let isolated = is_general.then(|| claude_subdirectory.read(cx).value().to_string());
                                    claude_view.update(cx, |this, cx| this.create_session(Agent::Claude, cwd, isolated, cx));
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("create-codex-session")
                                .primary()
                                .icon(IconName::SquareTerminal)
                                .label("Codex")
                                .on_click(move |_, window, cx| {
                                    let cwd = codex_directory.read(cx).value().to_string();
                                    let isolated = is_general.then(|| codex_subdirectory.read(cx).value().to_string());
                                    codex_view.update(cx, |this, cx| this.create_session(Agent::Codex, cwd, isolated, cx));
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    fn create_session(&mut self, agent: Agent, cwd: String, isolated_subdirectory: Option<String>, cx: &mut Context<Self>) {
        let cwd = cwd.trim().to_string();
        if cwd.is_empty() {
            self.status_message = localized(matches!(self.preferences.locale, Locale::SimplifiedChinese), "Choose a working directory", "请选择工作目录").into();
            cx.notify();
            return;
        }
        let isolated_subdirectory = isolated_subdirectory.map(|name| name.trim().to_string()).filter(|name| !name.is_empty());
        if let Some(name) = &isolated_subdirectory
            && (name == "." || name == ".." || name.contains('/') || name.contains('\\'))
        {
            self.status_message = localized(
                matches!(self.preferences.locale, Locale::SimplifiedChinese),
                "The isolated subdirectory name is invalid",
                "隔离子目录名称无效",
            )
            .into();
            cx.notify();
            return;
        }
        let request = CreateSessionRequest {
            agent,
            title: String::new(),
            cwd: cwd.clone(),
            rows: 36,
            cols: 120,
            resume: false,
            resume_id: None,
        };
        let Some(name) = isolated_subdirectory else {
            self.submit_session_request(request, cx);
            return;
        };
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.status_message = localized(
            matches!(self.preferences.locale, Locale::SimplifiedChinese),
            "Creating isolated directory…",
            "正在创建隔离目录…",
        )
        .into();
        self.runtime.spawn(async move {
            let result = api.create_directory(&profile, credential.as_deref().map(|value| value.as_str()), &cwd, &name).await;
            let _ = tx.send(result).await;
        });
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(directory) => {
                        let mut request = request;
                        request.cwd = directory.path;
                        this.submit_session_request(request, cx);
                    }
                    Err(error) => {
                        this.status_message = error.to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn open_directory_picker(&mut self, target: DirectoryPickerTarget, initial_path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.directory_picker = Some(DirectoryPickerState {
            target,
            listing: None,
            show_hidden: false,
            loading: false,
            error: None,
            request_generation: 0,
        });
        self.load_directory(initial_path, false, window, cx);
    }

    fn load_directory(&mut self, path: String, show_hidden: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.directory_picker.as_mut() else {
            return;
        };
        picker.request_generation = picker.request_generation.wrapping_add(1);
        let request_generation = picker.request_generation;
        picker.show_hidden = show_hidden;
        picker.loading = true;
        picker.error = None;
        self.directory_path.update(cx, |input, cx| input.set_value(path.clone(), window, cx));

        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let requested = (!path.trim().is_empty()).then_some(path);
        let (tx, rx) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            let result = api
                .directories(&profile, credential.as_deref().map(|value| value.as_str()), requested.as_deref(), show_hidden)
                .await;
            let _ = tx.send((request_generation, result)).await;
        });
        cx.spawn(async move |this, cx| {
            if let Ok((request_generation, result)) = rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    let Some(picker) = this.directory_picker.as_mut() else {
                        return;
                    };
                    if picker.request_generation != request_generation {
                        return;
                    }
                    picker.loading = false;
                    match result {
                        Ok(listing) => {
                            picker.listing = Some(listing);
                            picker.error = None;
                        }
                        Err(error) => picker.error = Some(error.to_string()),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn cancel_directory_picker(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.directory_picker.take().map(|picker| picker.target);
        self.resume_after_directory_picker(target, false, window, cx);
    }

    fn choose_directory(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.directory_picker.take() else {
            return;
        };
        let Some(path) = picker.listing.as_ref().map(|listing| listing.path.clone()) else {
            self.directory_picker = Some(picker);
            return;
        };
        match &picker.target {
            DirectoryPickerTarget::NewSession(_) => {
                self.new_session_directory.update(cx, |input, cx| input.set_value(path, window, cx));
            }
            DirectoryPickerTarget::Project(_) => {
                self.project_path.update(cx, |input, cx| input.set_value(path, window, cx));
            }
            DirectoryPickerTarget::GeneralRoot => {
                self.general_root.update(cx, |input, cx| input.set_value(path.clone(), window, cx));
                self.update_general_root(path, cx);
            }
        }
        self.resume_after_directory_picker(Some(picker.target), true, window, cx);
    }

    fn resume_after_directory_picker(&mut self, target: Option<DirectoryPickerTarget>, chosen: bool, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Some(DirectoryPickerTarget::NewSession(mode)) => self.show_new_session_dialog(mode, window, cx),
            Some(DirectoryPickerTarget::Project(project)) => self.show_project_editor(project, window, cx),
            Some(DirectoryPickerTarget::GeneralRoot) if !chosen => self.show_settings_dialog(window, cx),
            Some(DirectoryPickerTarget::GeneralRoot) | None => {}
        }
    }

    fn directory_picker_overlay(&self, cx: &mut Context<Self>) -> Div {
        let Some(picker) = &self.directory_picker else {
            return div();
        };
        let listing = picker.listing.clone();
        let chinese = matches!(self.preferences.locale, Locale::SimplifiedChinese);
        let show_hidden = picker.show_hidden;
        let loading = picker.loading;
        let error = picker.error.clone();
        let path_input = self.directory_path.clone();
        let mut entries = v_flex().gap_1();
        if let Some(listing) = &listing {
            for (index, entry) in listing.entries.iter().cloned().enumerate() {
                let view = cx.entity();
                entries = entries.child(
                    Button::new(("directory-entry", index))
                        .ghost()
                        .icon(IconName::Folder)
                        .label(entry.name)
                        .on_click(move |_, window, cx| {
                            view.update(cx, |this, cx| this.load_directory(entry.path.clone(), show_hidden, window, cx));
                        }),
                );
            }
        }
        let go_view = cx.entity();
        let go_path = path_input.clone();
        let hidden_view = cx.entity();
        let hidden_path = path_input.clone();
        let home_view = cx.entity();
        let parent_view = cx.entity();
        let home = listing.as_ref().and_then(|listing| listing.home.clone());
        let parent = listing.as_ref().and_then(|listing| listing.parent.clone());
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .bg(rgb(0x000000).opacity(0.42))
            .v_flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(720.))
                    .max_w(relative(0.92))
                    .max_h(relative(0.86))
                    .gap_3()
                    .p_5()
                    .rounded_lg()
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .shadow_lg()
                    .child(div().text_lg().font_semibold().child(localized(chinese, "Choose directory", "选择目录")))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("directory-hidden")
                                    .outline()
                                    .icon(if show_hidden { IconName::EyeOff } else { IconName::Eye })
                                    .label(if show_hidden {
                                        localized(chinese, "Hide hidden", "隐藏隐藏目录")
                                    } else {
                                        localized(chinese, "Show hidden", "显示隐藏目录")
                                    })
                                    .on_click(move |_, window, cx| {
                                        let path = hidden_path.read(cx).value().to_string();
                                        hidden_view.update(cx, |this, cx| this.load_directory(path, !show_hidden, window, cx));
                                    }),
                            )
                            .child(
                                Button::new("directory-home")
                                    .outline()
                                    .icon(IconName::HardDrive)
                                    .label(localized(chinese, "Home", "主目录"))
                                    .disabled(home.is_none())
                                    .on_click(move |_, window, cx| {
                                        if let Some(path) = &home {
                                            home_view.update(cx, |this, cx| this.load_directory(path.clone(), show_hidden, window, cx));
                                        }
                                    }),
                            )
                            .child(
                                Button::new("directory-parent")
                                    .outline()
                                    .icon(IconName::ArrowLeft)
                                    .label(localized(chinese, "Parent", "上级目录"))
                                    .disabled(parent.is_none())
                                    .on_click(move |_, window, cx| {
                                        if let Some(path) = &parent {
                                            parent_view.update(cx, |this, cx| this.load_directory(path.clone(), show_hidden, window, cx));
                                        }
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(&path_input).w_full())
                            .child(
                                Button::new("directory-go")
                                    .outline()
                                    .label(localized(chinese, "Go", "前往"))
                                    .on_click(move |_, window, cx| {
                                        let path = go_path.read(cx).value().to_string();
                                        go_view.update(cx, |this, cx| this.load_directory(path, show_hidden, window, cx));
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .h(px(390.))
                            .overflow_y_scrollbar()
                            .rounded_md()
                            .border_1()
                            .border_color(cx.theme().border)
                            .p_1()
                            .when(loading, |this| this.child(div().p_3().child(localized(chinese, "Loading…", "正在加载…"))))
                            .when_some(error, |this, error| this.child(div().p_3().text_color(cx.theme().danger).child(error)))
                            .when(!loading && listing.as_ref().is_some_and(|listing| listing.entries.is_empty()), |this| {
                                this.child(div().p_3().child(localized(chinese, "No subdirectories", "没有子目录")))
                            })
                            .child(entries),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("cancel-directory-picker")
                                    .outline()
                                    .label(localized(chinese, "Cancel", "取消"))
                                    .on_click(cx.listener(Self::cancel_directory_picker)),
                            )
                            .child(
                                Button::new("choose-directory")
                                    .primary()
                                    .label(localized(chinese, "Use directory", "使用此目录"))
                                    .disabled(listing.is_none())
                                    .on_click(cx.listener(Self::choose_directory)),
                            ),
                    ),
            )
    }

    fn submit_session_request(&mut self, request: CreateSessionRequest, cx: &mut Context<Self>) {
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.status_message = localized(matches!(self.preferences.locale, Locale::SimplifiedChinese), "Starting session…", "正在启动会话…").into();
        self.runtime.spawn(async move {
            let result = api.create_session(&profile, credential.as_deref().map(|value| value.as_str()), &request).await;
            let _ = tx.send(result).await;
        });
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(session) => {
                        let session_id = session.id.clone();
                        this.sessions.retain(|existing| existing.id != session.id);
                        this.sessions.push(session);
                        this.status_message = localized(matches!(this.preferences.locale, Locale::SimplifiedChinese), "Connected", "已连接").into();
                        this.select_session(session_id, cx);
                    }
                    Err(error) => {
                        this.status_message = error.to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn resume_history(&mut self, item: HistoryItem, cx: &mut Context<Self>) {
        if let Some(existing) = self
            .sessions
            .iter()
            .filter(|session| session.agent == item.agent && session.native_session_id.as_deref() == Some(item.id.as_str()))
            .filter(|session| session.status != akmux_client_core::SessionStatus::Exited)
            .max_by_key(|session| session.created_at_ms)
        {
            self.select_session(existing.id.clone(), cx);
            return;
        }
        self.submit_session_request(
            CreateSessionRequest {
                agent: item.agent,
                title: item.title,
                cwd: item.cwd,
                rows: 36,
                cols: 120,
                resume: false,
                resume_id: Some(item.id),
            },
            cx,
        );
    }

    fn save_preferences(&mut self, cx: &mut Context<Self>) {
        self.close_to_tray.store(matches!(self.preferences.close_behavior, CloseBehavior::Tray), Ordering::Relaxed);
        if let Err(error) = self.state_store.save_preferences(&self.preferences) {
            self.status_message = error.to_string();
        }
        cx.notify();
    }

    fn refresh_tray_labels(&mut self) {
        let sessions = self.sessions.iter().map(|session| (session.id.clone(), session.title.clone())).collect::<Vec<_>>();
        let _ = self.tray.update_sessions(&sessions, matches!(self.preferences.locale, Locale::SimplifiedChinese));
    }

    fn set_theme(&mut self, theme: PreferenceThemeMode, window: &mut Window, cx: &mut Context<Self>) {
        self.preferences.theme = theme;
        let mode = match self.preferences.theme {
            PreferenceThemeMode::Light => UIThemeMode::Light,
            PreferenceThemeMode::Dark => UIThemeMode::Dark,
            PreferenceThemeMode::System => window.appearance().into(),
        };
        UITheme::change(mode, Some(window), cx);
        self.apply_material(window);
        self.save_preferences(cx);
    }

    fn apply_material(&self, window: &mut Window) {
        apply_window_material(window, self.preferences.material_enabled);
    }

    fn toggle_material(&mut self, cx: &mut Context<Self>) {
        self.preferences.material_enabled = !self.preferences.material_enabled;
        let _ = self.window_handle.update(cx, |_, window, _| self.apply_material(window));
        self.save_preferences(cx);
    }

    fn adjust_material_transparency(&mut self, delta: i16, cx: &mut Context<Self>) {
        self.preferences.material_transparency = (i16::from(self.preferences.material_transparency) + delta).clamp(0, 100) as u8;
        self.save_preferences(cx);
    }

    fn adjust_terminal_font(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.preferences.terminal_font_size = (self.preferences.terminal_font_size + delta).clamp(9.0, 24.0);
        for terminal in self.terminals.values() {
            terminal.update(cx, |terminal, cx| terminal.set_font_size(self.preferences.terminal_font_size, cx));
        }
        self.save_preferences(cx);
    }

    fn adjust_sidebar_width(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.preferences.sidebar_width = (self.preferences.sidebar_width + delta).clamp(188.0, 420.0);
        self.save_preferences(cx);
    }

    fn open_settings(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let root_value = self.settings.as_ref().map(|settings| settings.general_root.as_str()).unwrap_or_default();
        self.general_root.update(cx, |input, cx| input.set_value(root_value, window, cx));
        self.show_settings_dialog(window, cx);
    }

    fn show_settings_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        let preferences = self.preferences.clone();
        let chinese = matches!(preferences.locale, Locale::SimplifiedChinese);
        let general_root = self.general_root.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let system = view.clone();
            let light = view.clone();
            let dark = view.clone();
            let smaller = view.clone();
            let larger = view.clone();
            let english = view.clone();
            let chinese_view = view.clone();
            let material = view.clone();
            let weaker_material = view.clone();
            let stronger_material = view.clone();
            let close_behavior = view.clone();
            let narrower_sidebar = view.clone();
            let wider_sidebar = view.clone();
            let save_root = view.clone();
            let root_input = general_root.clone();
            let browse_root = view.clone();
            let browse_root_input = general_root.clone();

            let appearance = h_flex()
                .gap_2()
                .child(
                    Button::new("theme-system")
                        .outline()
                        .selected(matches!(&preferences.theme, PreferenceThemeMode::System))
                        .label(localized(chinese, "System", "系统"))
                        .on_click(move |_, window, cx| {
                            system.update(cx, |this, cx| this.set_theme(PreferenceThemeMode::System, window, cx));
                        }),
                )
                .child(
                    Button::new("theme-light")
                        .outline()
                        .selected(matches!(&preferences.theme, PreferenceThemeMode::Light))
                        .label(localized(chinese, "Light", "亮色"))
                        .on_click(move |_, window, cx| {
                            light.update(cx, |this, cx| this.set_theme(PreferenceThemeMode::Light, window, cx));
                        }),
                )
                .child(
                    Button::new("theme-dark")
                        .outline()
                        .selected(matches!(&preferences.theme, PreferenceThemeMode::Dark))
                        .label(localized(chinese, "Dark", "暗色"))
                        .on_click(move |_, window, cx| {
                            dark.update(cx, |this, cx| this.set_theme(PreferenceThemeMode::Dark, window, cx));
                        }),
                );
            let font = h_flex()
                .gap_2()
                .items_center()
                .child(Button::new("font-smaller").outline().label("−").on_click(move |_, _, cx| {
                    smaller.update(cx, |this, cx| this.adjust_terminal_font(-1.0, cx));
                }))
                .child(div().min_w_20().text_center().child(format!("{} px", preferences.terminal_font_size)))
                .child(Button::new("font-larger").outline().label("+").on_click(move |_, _, cx| {
                    larger.update(cx, |this, cx| this.adjust_terminal_font(1.0, cx));
                }));
            let language = h_flex()
                .gap_2()
                .child(
                    Button::new("locale-en")
                        .outline()
                        .selected(matches!(&preferences.locale, Locale::English))
                        .label("English")
                        .on_click(move |_, _, cx| {
                            english.update(cx, |this, cx| {
                                this.preferences.locale = Locale::English;
                                for terminal in this.terminals.values() {
                                    terminal.update(cx, |terminal, cx| terminal.set_simplified_chinese(false, cx));
                                }
                                this.refresh_tray_labels();
                                this.save_preferences(cx);
                            });
                        }),
                )
                .child(
                    Button::new("locale-zh")
                        .outline()
                        .selected(matches!(&preferences.locale, Locale::SimplifiedChinese))
                        .label("简体中文")
                        .on_click(move |_, _, cx| {
                            chinese_view.update(cx, |this, cx| {
                                this.preferences.locale = Locale::SimplifiedChinese;
                                for terminal in this.terminals.values() {
                                    terminal.update(cx, |terminal, cx| terminal.set_simplified_chinese(true, cx));
                                }
                                this.refresh_tray_labels();
                                this.save_preferences(cx);
                            });
                        }),
                );
            let material_label = if preferences.material_enabled {
                localized(chinese, "Material on", "背景材质已开启")
            } else {
                localized(chinese, "Material off", "背景材质已关闭")
            };
            let material_button = Button::new("toggle-material")
                .outline()
                .selected(preferences.material_enabled)
                .label(material_label)
                .on_click(move |_, _, cx| {
                    material.update(cx, |this, cx| this.toggle_material(cx));
                });
            let material_strength = h_flex()
                .gap_2()
                .items_center()
                .child(Button::new("material-weaker").outline().label("−").on_click(move |_, _, cx| {
                    weaker_material.update(cx, |this, cx| this.adjust_material_transparency(-5, cx));
                }))
                .child(div().min_w_20().text_center().child(format!("{}%", preferences.material_transparency)))
                .child(Button::new("material-stronger").outline().label("+").on_click(move |_, _, cx| {
                    stronger_material.update(cx, |this, cx| this.adjust_material_transparency(5, cx));
                }));
            let close_label = match &preferences.close_behavior {
                CloseBehavior::Tray => localized(chinese, "Keep running in tray", "在托盘继续运行"),
                CloseBehavior::Quit => localized(chinese, "Quit application", "退出应用"),
            };
            let close_button = Button::new("close-behavior").outline().label(close_label).on_click(move |_, _, cx| {
                close_behavior.update(cx, |this, cx| {
                    this.preferences.close_behavior = match this.preferences.close_behavior {
                        CloseBehavior::Tray => CloseBehavior::Quit,
                        CloseBehavior::Quit => CloseBehavior::Tray,
                    };
                    this.save_preferences(cx);
                });
            });
            let sidebar_width = h_flex()
                .gap_2()
                .items_center()
                .child(Button::new("sidebar-narrower").outline().label("−").on_click(move |_, _, cx| {
                    narrower_sidebar.update(cx, |this, cx| this.adjust_sidebar_width(-16.0, cx));
                }))
                .child(div().min_w_20().text_center().child(format!("{} px", preferences.sidebar_width.round())))
                .child(Button::new("sidebar-wider").outline().label("+").on_click(move |_, _, cx| {
                    wider_sidebar.update(cx, |this, cx| this.adjust_sidebar_width(16.0, cx));
                }));

            dialog
                .title(localized(chinese, "Desktop settings", "桌面设置"))
                .w(px(620.))
                .child(
                    v_flex()
                        .gap_5()
                        .child(setting_group(localized(chinese, "Appearance", "外观"), appearance))
                        .child(material_button)
                        .child(setting_group(localized(chinese, "Material transparency", "材质透明度"), material_strength))
                        .child(setting_group(localized(chinese, "Terminal font", "终端字号"), font))
                        .child(setting_group(localized(chinese, "Sidebar width", "侧栏宽度"), sidebar_width))
                        .child(setting_group(localized(chinese, "Language", "语言"), language))
                        .child(setting_group(localized(chinese, "Window close", "关闭窗口时"), close_button))
                        .child(setting_group(
                            localized(chinese, "General workspace root", "通用工作区根目录"),
                            h_flex()
                                .gap_2()
                                .child(Input::new(&general_root).w_full())
                                .child(
                                    Button::new("browse-general-root")
                                        .outline()
                                        .icon(IconName::Folder)
                                        .label(localized(chinese, "Browse", "浏览"))
                                        .on_click(move |_, window, cx| {
                                            let initial = browse_root_input.read(cx).value().to_string();
                                            window.close_dialog(cx);
                                            browse_root.update(cx, |this, cx| this.open_directory_picker(DirectoryPickerTarget::GeneralRoot, initial, window, cx));
                                        }),
                                )
                                .child(
                                    Button::new("save-general-root")
                                        .outline()
                                        .label(localized(chinese, "Save", "保存"))
                                        .on_click(move |_, _, cx| {
                                            let root = root_input.read(cx).value().trim().to_string();
                                            save_root.update(cx, |this, cx| this.update_general_root(root, cx));
                                        }),
                                ),
                        )),
                )
                .footer(DialogFooter::new().child(DialogClose::new().child(Button::new("close-settings").primary().label(localized(chinese, "Done", "完成")))))
        });
    }

    fn update_general_root(&mut self, root: String, cx: &mut Context<Self>) {
        if root.is_empty() {
            self.status_message = localized(
                matches!(self.preferences.locale, Locale::SimplifiedChinese),
                "General workspace root cannot be empty",
                "通用工作区根目录不能为空",
            )
            .into();
            cx.notify();
            return;
        }
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.status_message = localized(
            matches!(self.preferences.locale, Locale::SimplifiedChinese),
            "Updating workspace root…",
            "正在更新工作区根目录…",
        )
        .into();
        self.runtime.spawn(async move {
            let result = api
                .update_settings(&profile, credential.as_deref().map(|value| value.as_str()), &serde_json::json!({ "general_root": root }))
                .await;
            let _ = tx.send(result).await;
        });
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(settings) => {
                        this.settings = Some(settings);
                        this.status_message = localized(matches!(this.preferences.locale, Locale::SimplifiedChinese), "Connected", "已连接").into();
                        this.start_polling(cx);
                    }
                    Err(error) => {
                        this.status_message = error.to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn open_project_manager(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        let projects = self.settings.as_ref().map(|settings| settings.projects.clone()).unwrap_or_default();
        let directories = self
            .workspaces
            .as_ref()
            .map(|workspaces| workspaces.general.iter().chain(&workspaces.other).map(|group| group.path.clone()).collect::<Vec<_>>())
            .unwrap_or_default();
        let project_ids = projects.iter().map(|project| project.id.clone()).collect::<Vec<_>>();
        let other_directory_ids = self
            .settings
            .as_ref()
            .map(|settings| settings.other_directories.iter().map(|directory| directory.path.clone()).collect::<Vec<_>>())
            .unwrap_or_default();
        let sorts = self
            .settings
            .as_ref()
            .map(|settings| (settings.project_sort, settings.general_sort, settings.other_sort))
            .unwrap_or((SortMode::Priority, SortMode::Priority, SortMode::Priority));
        let mut manual_groups = Vec::new();
        if let (Some(workspaces), Some(settings)) = (&self.workspaces, &self.settings) {
            if settings.project_sort == SortMode::Manual {
                manual_groups.extend(
                    workspaces
                        .projects
                        .iter()
                        .map(|group| (format!("project:{}", group.project.id), group.project.name.clone(), group.history.clone())),
                );
            }
            manual_groups.extend(workspaces.general.iter().filter_map(|group| {
                let mode = settings.directory_sort.get(&group.path).copied().unwrap_or(settings.general_sort);
                (mode == SortMode::Manual).then(|| (format!("directory:{}", group.path), group.path.clone(), group.items.clone()))
            }));
            manual_groups.extend(workspaces.other.iter().filter_map(|group| {
                let mode = settings.directory_sort.get(&group.path).copied().unwrap_or(settings.other_sort);
                (mode == SortMode::Manual).then(|| (format!("directory:{}", group.path), group.path.clone(), group.items.clone()))
            }));
        }
        window.open_dialog(cx, move |dialog, _, _| {
            let mut list = v_flex().gap_1();
            for (index, project) in projects.iter().cloned().enumerate() {
                let edit_view = view.clone();
                let edit_project = project.clone();
                let session_view = view.clone();
                let session_path = project.path.clone();
                let icon_view = view.clone();
                let icon_key = format!("project:{}", project.id);
                let pin_view = view.clone();
                let pin_project = project.clone();
                let delete_view = view.clone();
                let delete_id = project.id.clone();
                let move_up_view = view.clone();
                let move_up_id = project.id.clone();
                let move_up_ids = project_ids.clone();
                let move_down_view = view.clone();
                let move_down_id = project.id.clone();
                let move_down_ids = project_ids.clone();
                list = list.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(Icon::new(IconName::Folder))
                        .child(v_flex().flex_1().min_w_0().child(project.name.clone()).child(div().text_xs().child(project.path.clone())))
                        .child(
                            Button::new(("new-project-session", index))
                                .outline()
                                .small()
                                .icon(IconName::Plus)
                                .label("New session")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    session_view.update(cx, |this, cx| this.open_project_session(session_path.clone(), window, cx));
                                }),
                        )
                        .child(Button::new(("edit-project", index)).outline().small().label("Edit").on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            edit_view.update(cx, |this, cx| this.open_project_editor(Some(edit_project.clone()), window, cx));
                        }))
                        .child(Button::new(("icon-project", index)).ghost().small().label("Icon").on_click(move |_, _, cx| {
                            icon_view.update(cx, |this, cx| this.cycle_workspace_icon(icon_key.clone(), cx));
                        }))
                        .child(
                            Button::new(("pin-project", index))
                                .ghost()
                                .small()
                                .label(if project.pinned { "Unpin" } else { "Pin" })
                                .on_click(move |_, _, cx| {
                                    pin_view.update(cx, |this, cx| {
                                        this.update_project(pin_project.id.clone(), serde_json::json!({ "pinned": !pin_project.pinned }), cx)
                                    });
                                }),
                        )
                        .child(
                            Button::new(("remove-project", index))
                                .danger()
                                .small()
                                .icon(IconName::Delete)
                                .label("Remove")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    delete_view.update(cx, |this, cx| this.confirm_delete_project(delete_id.clone(), window, cx));
                                }),
                        )
                        .child(
                            Button::new(("move-project-up", index))
                                .ghost()
                                .small()
                                .label("↑")
                                .disabled(index == 0)
                                .on_click(move |_, _, cx| {
                                    move_up_view.update(cx, |this, cx| {
                                        this.move_workspace_item("projects", "projects".into(), &move_up_id, move_up_ids.clone(), -1, cx)
                                    });
                                }),
                        )
                        .child(
                            Button::new(("move-project-down", index))
                                .ghost()
                                .small()
                                .label("↓")
                                .disabled(index + 1 == project_ids.len())
                                .on_click(move |_, _, cx| {
                                    move_down_view.update(cx, |this, cx| {
                                        this.move_workspace_item("projects", "projects".into(), &move_down_id, move_down_ids.clone(), 1, cx)
                                    });
                                }),
                        ),
                );
            }
            let mut directory_list = v_flex().gap_1();
            for (index, path) in directories.iter().cloned().enumerate() {
                let icon_view = view.clone();
                let icon_key = format!("directory:{path}");
                let sort_view = view.clone();
                let sort_path = path.clone();
                let move_up_view = view.clone();
                let move_up_path = path.clone();
                let move_up_ids = other_directory_ids.clone();
                let move_down_view = view.clone();
                let move_down_path = path.clone();
                let move_down_ids = other_directory_ids.clone();
                let other_index = other_directory_ids.iter().position(|candidate| candidate == &path);
                directory_list = directory_list.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(Icon::new(IconName::Folder))
                        .child(div().flex_1().min_w_0().text_xs().child(path))
                        .child(Button::new(("icon-directory", index)).ghost().small().label("Change icon").on_click(move |_, _, cx| {
                            icon_view.update(cx, |this, cx| this.cycle_workspace_icon(icon_key.clone(), cx));
                        }))
                        .child(Button::new(("sort-directory", index)).ghost().small().label("Sort").on_click(move |_, _, cx| {
                            sort_view.update(cx, |this, cx| this.cycle_directory_sort(sort_path.clone(), cx));
                        }))
                        .when_some(other_index, |this, other_index| {
                            this.child(
                                Button::new(("move-directory-up", index))
                                    .ghost()
                                    .small()
                                    .label("↑")
                                    .disabled(other_index == 0)
                                    .on_click(move |_, _, cx| {
                                        move_up_view.update(cx, |this, cx| {
                                            this.move_workspace_item("directories", "other".into(), &move_up_path, move_up_ids.clone(), -1, cx)
                                        });
                                    }),
                            )
                            .child(
                                Button::new(("move-directory-down", index))
                                    .ghost()
                                    .small()
                                    .label("↓")
                                    .disabled(other_index + 1 == move_down_ids.len())
                                    .on_click(move |_, _, cx| {
                                        move_down_view.update(cx, |this, cx| {
                                            this.move_workspace_item("directories", "other".into(), &move_down_path, move_down_ids.clone(), 1, cx)
                                        });
                                    }),
                            )
                        }),
                );
            }
            let mut manual_session_list = v_flex().gap_2();
            for (group_index, (scope, group_label, items)) in manual_groups.iter().cloned().enumerate() {
                let ids = items
                    .iter()
                    .map(|item| format!("{}:{}", agent_name(item.agent).to_lowercase().replace(" code", ""), item.id))
                    .collect::<Vec<_>>();
                let mut group = v_flex().gap_1().child(div().text_xs().font_semibold().child(group_label));
                for (item_index, item) in items.into_iter().enumerate() {
                    let item_id = ids[item_index].clone();
                    let up_view = view.clone();
                    let up_scope = scope.clone();
                    let up_ids = ids.clone();
                    let up_id = item_id.clone();
                    let down_view = view.clone();
                    let down_scope = scope.clone();
                    let down_ids = ids.clone();
                    let down_id = item_id;
                    group = group.child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().min_w_0().text_xs().child(item.title))
                            .child(
                                Button::new(("move-session-up", group_index * 2_000 + item_index))
                                    .ghost()
                                    .small()
                                    .label("↑")
                                    .disabled(item_index == 0)
                                    .on_click(move |_, _, cx| {
                                        up_view.update(cx, |this, cx| this.move_workspace_item("sessions", up_scope.clone(), &up_id, up_ids.clone(), -1, cx));
                                    }),
                            )
                            .child(
                                Button::new(("move-session-down", group_index * 2_000 + item_index))
                                    .ghost()
                                    .small()
                                    .label("↓")
                                    .disabled(item_index + 1 == down_ids.len())
                                    .on_click(move |_, _, cx| {
                                        down_view.update(cx, |this, cx| this.move_workspace_item("sessions", down_scope.clone(), &down_id, down_ids.clone(), 1, cx));
                                    }),
                            ),
                    );
                }
                manual_session_list = manual_session_list.child(group);
            }
            let has_manual_sessions = !manual_groups.is_empty();
            let add_view = view.clone();
            let project_sort_view = view.clone();
            let general_sort_view = view.clone();
            let other_sort_view = view.clone();
            dialog
                .title("Projects")
                .w(px(700.))
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("cycle-project-sort")
                                        .outline()
                                        .small()
                                        .label(format!("Projects: {:?}", sorts.0))
                                        .on_click(move |_, _, cx| {
                                            project_sort_view.update(cx, |this, cx| this.cycle_sort("projects", cx));
                                        }),
                                )
                                .child(
                                    Button::new("cycle-general-sort")
                                        .outline()
                                        .small()
                                        .label(format!("General: {:?}", sorts.1))
                                        .on_click(move |_, _, cx| {
                                            general_sort_view.update(cx, |this, cx| this.cycle_sort("general", cx));
                                        }),
                                )
                                .child(
                                    Button::new("cycle-other-sort")
                                        .outline()
                                        .small()
                                        .label(format!("Other: {:?}", sorts.2))
                                        .on_click(move |_, _, cx| {
                                            other_sort_view.update(cx, |this, cx| this.cycle_sort("other", cx));
                                        }),
                                ),
                        )
                        .child(list.max_h(px(300.)).overflow_y_scrollbar())
                        .child(div().text_sm().font_semibold().child("Directories"))
                        .child(directory_list.max_h(px(160.)).overflow_y_scrollbar())
                        .when(has_manual_sessions, |this| {
                            this.child(div().text_sm().font_semibold().child("Manual session order"))
                                .child(manual_session_list.max_h(px(220.)).overflow_y_scrollbar())
                        }),
                )
                .footer(
                    DialogFooter::new()
                        .gap_2()
                        .child(DialogClose::new().child(Button::new("close-project-manager").outline().label("Close")))
                        .child(
                            Button::new("add-project")
                                .primary()
                                .icon(IconName::Plus)
                                .label("Add project")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    add_view.update(cx, |this, cx| this.open_project_editor(None, window, cx));
                                }),
                        ),
                )
        });
    }

    fn cycle_sort(&mut self, section: &'static str, cx: &mut Context<Self>) {
        let Some(settings) = &self.settings else {
            return;
        };
        let current = match section {
            "projects" => settings.project_sort,
            "general" => settings.general_sort,
            _ => settings.other_sort,
        };
        let next = match current {
            SortMode::Priority => SortMode::Recent,
            SortMode::Recent => SortMode::Manual,
            SortMode::Manual => SortMode::Priority,
        };
        let key = match section {
            "projects" => "project_sort",
            "general" => "general_sort",
            _ => "other_sort",
        };
        let mut patch = serde_json::Map::new();
        patch.insert(key.into(), serde_json::to_value(next).expect("sort mode is serializable"));
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            let _ = tx
                .send(
                    api.update_settings(&profile, credential.as_deref().map(|value| value.as_str()), &serde_json::Value::Object(patch))
                        .await,
                )
                .await;
        });
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(settings) => {
                        this.settings = Some(settings);
                        this.start_polling(cx);
                    }
                    Err(error) => {
                        this.status_message = error.to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn cycle_workspace_icon(&mut self, key: String, cx: &mut Context<Self>) {
        let next = match self.preferences.workspace_icons.get(&key).map(String::as_str) {
            None | Some("folder") => "star",
            Some("star") => "terminal",
            _ => "folder",
        };
        self.preferences.workspace_icons.insert(key, next.into());
        self.save_preferences(cx);
    }

    fn cycle_directory_sort(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(settings) = &self.settings else {
            return;
        };
        let fallback = if self.workspaces.as_ref().is_some_and(|workspaces| workspaces.general.iter().any(|group| group.path == path)) {
            settings.general_sort
        } else {
            settings.other_sort
        };
        let current = settings.directory_sort.get(&path).copied().unwrap_or(fallback);
        let next = match current {
            SortMode::Priority => SortMode::Recent,
            SortMode::Recent => SortMode::Manual,
            SortMode::Manual => SortMode::Priority,
        };
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            let patch = serde_json::json!({ "directory_sort": { "path": path, "mode": next } });
            let _ = tx
                .send(api.update_settings(&profile, credential.as_deref().map(|value| value.as_str()), &patch).await)
                .await;
        });
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(settings) => {
                        this.settings = Some(settings);
                        this.start_polling(cx);
                    }
                    Err(error) => {
                        this.status_message = error.to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn move_workspace_item(&mut self, kind: &'static str, scope: String, id: &str, mut ids: Vec<String>, delta: isize, cx: &mut Context<Self>) {
        let Some(index) = ids.iter().position(|candidate| candidate == id) else {
            return;
        };
        let destination = index.saturating_add_signed(delta).min(ids.len().saturating_sub(1));
        if destination == index {
            return;
        }
        ids.swap(index, destination);
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            let _ = tx
                .send(api.reorder(&profile, credential.as_deref().map(|value| value.as_str()), kind, &scope, &ids).await)
                .await;
        });
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(settings) => {
                        this.settings = Some(settings);
                        this.start_polling(cx);
                    }
                    Err(error) => {
                        this.status_message = error.to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn open_project_editor(&mut self, project: Option<Project>, window: &mut Window, cx: &mut Context<Self>) {
        let name_value = project.as_ref().map(|project| project.name.as_str()).unwrap_or_default();
        let path_value = project.as_ref().map(|project| project.path.as_str()).unwrap_or_default();
        self.project_name.update(cx, |input, cx| input.set_value(name_value, window, cx));
        self.project_path.update(cx, |input, cx| input.set_value(path_value, window, cx));
        self.show_project_editor(project, window, cx);
    }

    fn show_project_editor(&mut self, project: Option<Project>, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.project_name.clone();
        let path = self.project_path.clone();
        let project_id = project.as_ref().map(|project| project.id.clone());
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let save_view = view.clone();
            let save_name = name.clone();
            let save_path = path.clone();
            let save_id = project_id.clone();
            let browse_view = view.clone();
            let browse_path = path.clone();
            let browse_project = project.clone();
            dialog
                .title(if project_id.is_some() { "Edit project" } else { "Add project" })
                .w(px(560.))
                .child(
                    v_flex().gap_3().child(Input::new(&name).w_full()).child(
                        h_flex().gap_2().child(Input::new(&path).w_full()).child(
                            Button::new("browse-project-directory")
                                .outline()
                                .icon(IconName::Folder)
                                .label("Browse")
                                .on_click(move |_, window, cx| {
                                    let initial = browse_path.read(cx).value().to_string();
                                    window.close_dialog(cx);
                                    browse_view.update(cx, |this, cx| {
                                        this.open_directory_picker(DirectoryPickerTarget::Project(browse_project.clone()), initial, window, cx)
                                    });
                                }),
                        ),
                    ),
                )
                .footer(
                    DialogFooter::new()
                        .gap_2()
                        .child(DialogClose::new().child(Button::new("cancel-project-editor").outline().label("Cancel")))
                        .child(Button::new("save-project").primary().label("Save").on_click(move |_, window, cx| {
                            let name = save_name.read(cx).value().trim().to_string();
                            let path = save_path.read(cx).value().trim().to_string();
                            save_view.update(cx, |this, cx| this.save_project(save_id.clone(), name, path, cx));
                            window.close_dialog(cx);
                        })),
                )
        });
    }

    fn save_project(&mut self, id: Option<String>, name: String, path: String, cx: &mut Context<Self>) {
        if name.is_empty() || path.is_empty() {
            self.status_message = localized(
                matches!(self.preferences.locale, Locale::SimplifiedChinese),
                "Project name and path are required",
                "项目名称和路径为必填项",
            )
            .into();
            cx.notify();
            return;
        }
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.status_message = localized(matches!(self.preferences.locale, Locale::SimplifiedChinese), "Saving project…", "正在保存项目…").into();
        self.runtime.spawn(async move {
            let result = match id {
                Some(id) => {
                    api.update_project(
                        &profile,
                        credential.as_deref().map(|value| value.as_str()),
                        &id,
                        &serde_json::json!({ "name": name, "path": path }),
                    )
                    .await
                }
                None => api.create_project(&profile, credential.as_deref().map(|value| value.as_str()), &path, &name).await,
            };
            let _ = tx.send(result).await;
        });
        self.finish_project_change(rx, cx);
    }

    fn update_project(&mut self, id: String, patch: serde_json::Value, cx: &mut Context<Self>) {
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            let _ = tx
                .send(api.update_project(&profile, credential.as_deref().map(|value| value.as_str()), &id, &patch).await)
                .await;
        });
        self.finish_project_change(rx, cx);
    }

    fn finish_project_change(&mut self, rx: async_channel::Receiver<akmux_client_core::Result<Project>>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(_) => this.start_polling(cx),
                    Err(error) => {
                        this.status_message = error.to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn delete_project(&mut self, id: String, cx: &mut Context<Self>) {
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            let _ = tx.send(api.delete_project(&profile, credential.as_deref().map(|value| value.as_str()), &id).await).await;
        });
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(()) => this.start_polling(cx),
                    Err(error) => {
                        this.status_message = error.to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn confirm_delete_project(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let project_name = self
            .settings
            .as_ref()
            .and_then(|settings| settings.projects.iter().find(|project| project.id == id))
            .map(|project| project.name.clone())
            .unwrap_or_else(|| "this project".into());
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let delete_view = view.clone();
            let delete_id = id.clone();
            dialog
                .title("Remove project?")
                .child(format!("{project_name} will be removed from the workspace. Existing sessions and files are not deleted."))
                .footer(
                    DialogFooter::new()
                        .gap_2()
                        .child(DialogClose::new().child(Button::new("cancel-delete-project").outline().label("Cancel")))
                        .child(Button::new("confirm-delete-project").danger().label("Remove project").on_click(move |_, window, cx| {
                            delete_view.update(cx, |this, cx| this.delete_project(delete_id.clone(), cx));
                            window.close_dialog(cx);
                        })),
                )
        });
    }

    fn open_backend_manager(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.backend_editing_profile = None;
        self.backend_name.update(cx, |input, cx| input.set_value("", window, cx));
        self.backend_address.update(cx, |input, cx| input.set_value("", window, cx));
        self.backend_pairing_link.update(cx, |input, cx| input.set_value("", window, cx));
        let view = cx.entity();
        let profiles = self.profiles.clone();
        let name = self.backend_name.clone();
        let address = self.backend_address.clone();
        let pairing = self.backend_pairing_link.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let mut profile_list = v_flex().gap_1();
            for (index, profile) in profiles.profiles.iter().enumerate() {
                let select_view = view.clone();
                let select_id = profile.id.clone();
                let refresh_view = view.clone();
                let refresh_id = profile.id.clone();
                let move_up_view = view.clone();
                let move_up_id = profile.id.clone();
                let move_down_view = view.clone();
                let move_down_id = profile.id.clone();
                let edit_view = view.clone();
                let edit_profile = profile.clone();
                let mut row = h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new(("select-backend", index))
                            .outline()
                            .selected(profile.id == profiles.active_profile_id)
                            .icon(if profile.kind == BackendKind::Local { IconName::HardDrive } else { IconName::Network })
                            .label(profile.name.clone())
                            .on_click(move |_, window, cx| {
                                select_view.update(cx, |this, cx| this.apply_backend_intent(BackendProfileIntent::Select { profile_id: select_id.clone() }, cx));
                                window.close_dialog(cx);
                            }),
                    )
                    .child(div().flex_1().min_w_0().text_xs().child(profile.address.clone()))
                    .child(
                        Button::new(("refresh-backend", index))
                            .ghost()
                            .small()
                            .icon(IconName::Redo)
                            .label("Refresh")
                            .on_click(move |_, _, cx| {
                                refresh_view.update(cx, |this, cx| {
                                    this.apply_backend_intent(BackendProfileIntent::Refresh { profile_id: refresh_id.clone() }, cx)
                                });
                            }),
                    )
                    .child(
                        Button::new(("move-backend-up", index))
                            .ghost()
                            .small()
                            .label("↑")
                            .disabled(index == 0)
                            .on_click(move |_, _, cx| {
                                move_up_view.update(cx, |this, cx| this.move_backend(&move_up_id, -1, cx));
                            }),
                    )
                    .child(
                        Button::new(("move-backend-down", index))
                            .ghost()
                            .small()
                            .label("↓")
                            .disabled(index + 1 == profiles.profiles.len())
                            .on_click(move |_, _, cx| {
                                move_down_view.update(cx, |this, cx| this.move_backend(&move_down_id, 1, cx));
                            }),
                    )
                    .child(Button::new(("edit-backend", index)).outline().small().label("Edit").on_click(move |_, window, cx| {
                        edit_view.update(cx, |this, cx| {
                            this.backend_editing_profile = Some(edit_profile.clone());
                            this.backend_name.update(cx, |input, cx| input.set_value(&edit_profile.name, window, cx));
                            this.backend_address.update(cx, |input, cx| input.set_value(&edit_profile.address, window, cx));
                            this.backend_pairing_link.update(cx, |input, cx| input.set_value("", window, cx));
                            cx.notify();
                        });
                    }));
                if profile.kind == BackendKind::Remote {
                    let delete_view = view.clone();
                    let delete_id = profile.id.clone();
                    row = row.child(
                        Button::new(("delete-backend", index))
                            .danger()
                            .icon(IconName::Delete)
                            .label("Delete")
                            .on_click(move |_, _, cx| {
                                delete_view.update(cx, |this, cx| this.apply_backend_intent(BackendProfileIntent::Delete { profile_id: delete_id.clone() }, cx));
                            }),
                    );
                }
                profile_list = profile_list.child(row);
            }
            let add_view = view.clone();
            let add_name = name.clone();
            let add_address = address.clone();
            let add_pairing = pairing.clone();
            dialog
                .title("Backend profiles")
                .w(px(680.))
                .child(
                    v_flex().gap_5().child(setting_group("Available", profile_list)).child(
                        v_flex()
                            .gap_2()
                            .child(div().font_semibold().child("Add remote backend"))
                            .child(Input::new(&name).w_full())
                            .child(Input::new(&address).w_full())
                            .child(Input::new(&pairing).w_full())
                            .child(
                                div()
                                    .text_xs()
                                    .child("Remote addresses require HTTPS. The pairing link is origin-bound and the device credential is stored by the operating system."),
                            ),
                    ),
                )
                .footer(
                    DialogFooter::new()
                        .gap_2()
                        .child(DialogClose::new().child(Button::new("close-backends").outline().label("Close")))
                        .child(Button::new("add-backend").primary().label("Pair and add").on_click(move |_, window, cx| {
                            let name = add_name.read(cx).value().trim().to_string();
                            let address = add_address.read(cx).value().trim().trim_end_matches('/').to_string();
                            let pairing_link = add_pairing.read(cx).value().trim().to_string();
                            let profile = BackendProfile {
                                id: format!("remote-{}", &operation_id()[..16]),
                                name,
                                kind: BackendKind::Remote,
                                address,
                                instance_id: None,
                                has_credential: false,
                                requires_auth: true,
                                capabilities: Vec::new(),
                            };
                            add_view.update(cx, |this, cx| {
                                let mut profile = profile.clone();
                                if let Some(existing) = this.backend_editing_profile.take() {
                                    profile.id = existing.id;
                                    profile.kind = existing.kind;
                                    profile.instance_id = existing.instance_id;
                                    profile.has_credential = existing.has_credential;
                                    profile.requires_auth = existing.requires_auth;
                                    profile.capabilities = existing.capabilities;
                                }
                                this.apply_backend_intent(
                                    BackendProfileIntent::Save {
                                        profile,
                                        pairing_link: Some(pairing_link),
                                    },
                                    cx,
                                )
                            });
                            window.close_dialog(cx);
                        })),
                )
        });
    }

    fn move_backend(&mut self, profile_id: &str, delta: isize, cx: &mut Context<Self>) {
        let mut ids = self.profiles.profiles.iter().map(|profile| profile.id.clone()).collect::<Vec<_>>();
        let Some(index) = ids.iter().position(|id| id == profile_id) else {
            return;
        };
        let destination = index.saturating_add_signed(delta).min(ids.len().saturating_sub(1));
        if destination != index {
            ids.swap(index, destination);
            self.apply_backend_intent(BackendProfileIntent::Reorder { profile_ids: ids }, cx);
        }
    }

    fn apply_backend_intent(&mut self, intent: BackendProfileIntent, cx: &mut Context<Self>) {
        let lifecycle = self.lifecycle.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.status_message = localized(
            matches!(self.preferences.locale, Locale::SimplifiedChinese),
            "Updating backend profile…",
            "正在更新后端配置…",
        )
        .into();
        self.runtime.spawn(async move {
            let _ = tx.send(lifecycle.apply(intent).await).await;
        });
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(outcome) => this.handle_backend_outcome(outcome, cx),
                    Err(error) => {
                        this.status_message = error.to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn handle_backend_outcome(&mut self, outcome: BackendLifecycleOutcome, cx: &mut Context<Self>) {
        let previous_active = self.profiles.active_profile_id.clone();
        match outcome {
            BackendLifecycleOutcome::Applied { state } => {
                self.profiles = state;
                self.pending_identity = None;
                self.status_message = localized(matches!(self.preferences.locale, Locale::SimplifiedChinese), "Connected", "已连接").into();
            }
            BackendLifecycleOutcome::AppliedWithWarning { state, warning } => {
                self.profiles = state;
                self.pending_identity = None;
                self.status_message = warning;
            }
            BackendLifecycleOutcome::IdentityConfirmationRequired {
                state,
                challenge_id,
                observed_instance_id,
                ..
            } => {
                self.profiles = state;
                self.pending_identity = Some((challenge_id, observed_instance_id));
                self.status_message = localized(
                    matches!(self.preferences.locale, Locale::SimplifiedChinese),
                    "Backend identity changed; confirmation required",
                    "后端身份已变化，需要确认",
                )
                .into();
            }
            BackendLifecycleOutcome::AuthenticationRequired { state, .. } => {
                self.profiles = state;
                self.status_message = localized(
                    matches!(self.preferences.locale, Locale::SimplifiedChinese),
                    "Backend authentication required",
                    "后端需要认证",
                )
                .into();
            }
            BackendLifecycleOutcome::Offline { state, .. } => {
                self.profiles = state;
                self.status_message = localized(matches!(self.preferences.locale, Locale::SimplifiedChinese), "Backend is offline", "后端离线").into();
            }
        }
        if previous_active != self.profiles.active_profile_id {
            self.active_session_id = None;
            self.terminals.clear();
            self.sessions.clear();
            self.workspaces = None;
            self.settings = None;
            self.start_polling(cx);
        }
        cx.notify();
    }

    fn confirm_identity(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((challenge_id, _)) = self.pending_identity.clone() {
            self.apply_backend_intent(BackendProfileIntent::ConfirmIdentity { challenge_id }, cx);
        }
    }

    fn cancel_identity(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((challenge_id, _)) = self.pending_identity.take() {
            self.apply_backend_intent(BackendProfileIntent::CancelIdentity { challenge_id }, cx);
        }
    }

    fn history_results(&self) -> Vec<HistoryItem> {
        let mut items = Vec::new();
        if let Some(workspaces) = &self.workspaces {
            items.extend(workspaces.projects.iter().flat_map(|project| project.history.iter().cloned()));
            items.extend(workspaces.general.iter().flat_map(|group| group.items.iter().cloned()));
            items.extend(workspaces.other.iter().flat_map(|group| group.items.iter().cloned()));
        }
        let query = self.search_query.trim().to_lowercase();
        let mut seen = std::collections::HashSet::new();
        items.retain(|item| {
            seen.insert((item.agent, item.id.clone())) && (query.is_empty() || format!("{} {} {:?}", item.title, item.cwd, item.agent).to_lowercase().contains(&query))
        });
        items.truncate(if query.is_empty() { 40 } else { 80 });
        items
    }

    fn open_search(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.search_query.clear();
        self.search_input.update(cx, |input, cx| input.set_value("", window, cx));
        let view = cx.entity();
        let input = self.search_input.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let results = view.read(cx).history_results();
            let mut list = v_flex().gap_1();
            for (index, item) in results.into_iter().enumerate() {
                let resume_view = view.clone();
                list = list.child(
                    Button::new(("history-result", index))
                        .ghost()
                        .label(format!("{}  ·  {}", item.title, item.cwd))
                        .icon(match item.agent {
                            Agent::Claude => IconName::Bot,
                            Agent::Codex => IconName::SquareTerminal,
                        })
                        .on_click(move |_, window, cx| {
                            resume_view.update(cx, |this, cx| this.resume_history(item.clone(), cx));
                            window.close_dialog(cx);
                        }),
                );
            }
            dialog
                .title("Search and resume")
                .w(px(680.))
                .child(v_flex().gap_3().child(Input::new(&input).w_full()).child(list.max_h(px(440.)).overflow_y_scrollbar()))
                .footer(DialogFooter::new().child(DialogClose::new().child(Button::new("close-search").outline().label("Close"))))
        });
    }

    fn session_menu(&self, cx: &mut Context<Self>) -> SidebarMenu {
        let mut menu = SidebarMenu::new();
        for session in &self.sessions {
            let session_id = session.id.clone();
            let mut label = if session.title.trim().is_empty() { session.cwd.clone() } else { session.title.clone() };
            if self.attention.contains_key(&session.id) {
                label = format!("● {label}");
            }
            let active = self.active_session_id.as_deref() == Some(session.id.as_str());
            menu = menu.child(
                SidebarMenuItem::new(label)
                    .icon(match session.agent {
                        akmux_client_core::Agent::Claude => IconName::Bot,
                        akmux_client_core::Agent::Codex => IconName::SquareTerminal,
                    })
                    .active(active)
                    .on_click(cx.listener(move |this, _, _, cx| this.select_session(session_id.clone(), cx))),
            );
        }
        menu
    }

    fn history_children(&self, items: &[HistoryItem], cx: &mut Context<Self>) -> Vec<SidebarMenuItem> {
        let mut children = Vec::new();
        for history in items.iter().take(40) {
            let item = history.clone();
            children.push(
                SidebarMenuItem::new(history.title.clone())
                    .icon(match history.agent {
                        Agent::Claude => IconName::Bot,
                        Agent::Codex => IconName::SquareTerminal,
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.resume_history(item.clone(), cx))),
            );
        }
        children
    }

    fn sidebar_group_open(&self, key: &str) -> bool {
        self.preferences
            .sidebar_expansion
            .get(&self.active_profile().id)
            .is_some_and(|expanded| expanded.iter().any(|candidate| candidate == key))
    }

    fn toggle_sidebar_group(&mut self, key: &str, cx: &mut Context<Self>) {
        let expanded = self.preferences.sidebar_expansion.entry(self.active_profile().id).or_default();
        if let Some(index) = expanded.iter().position(|candidate| candidate == key) {
            expanded.remove(index);
        } else {
            expanded.push(key.to_owned());
        }
        self.save_preferences(cx);
    }

    fn project_menu(&self, cx: &mut Context<Self>) -> SidebarMenu {
        let mut menu = SidebarMenu::new();
        if let Some(workspaces) = &self.workspaces {
            for project in &workspaces.projects {
                let key = format!("project:{}", project.project.id);
                let handler_key = key.clone();
                menu = menu.child(
                    SidebarMenuItem::new(project.project.name.clone())
                        .icon(self.workspace_icon(&key))
                        .default_open(self.sidebar_group_open(&key))
                        .click_to_toggle(true)
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_sidebar_group(&handler_key, cx)))
                        .children(self.history_children(&project.history, cx)),
                );
            }
        }
        menu
    }

    fn directory_menu(&self, other: bool, cx: &mut Context<Self>) -> SidebarMenu {
        let mut menu = SidebarMenu::new();
        if let Some(workspaces) = &self.workspaces {
            let groups = if other { &workspaces.other } else { &workspaces.general };
            for group in groups {
                let key = format!("directory:{}", group.path);
                let handler_key = key.clone();
                let label = std::path::Path::new(&group.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&group.path)
                    .to_owned();
                menu = menu.child(
                    SidebarMenuItem::new(label)
                        .icon(self.workspace_icon(&key))
                        .default_open(self.sidebar_group_open(&key))
                        .click_to_toggle(true)
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_sidebar_group(&handler_key, cx)))
                        .children(self.history_children(&group.items, cx)),
                );
            }
        }
        menu
    }

    fn workspace_icon(&self, key: &str) -> IconName {
        match self.preferences.workspace_icons.get(key).map(String::as_str) {
            Some("star") => IconName::Star,
            Some("terminal") => IconName::SquareTerminal,
            _ => IconName::Folder,
        }
    }

    fn active_session(&self) -> Option<&SessionInfo> {
        let id = self.active_session_id.as_deref()?;
        self.sessions.iter().find(|session| session.id == id)
    }

    fn session_tabs(&self, cx: &mut Context<Self>) -> Div {
        let mut tabs = div().h_flex().flex_1().min_w_0().gap_1().overflow_hidden();
        for (index, session) in self.sessions.iter().enumerate() {
            let id = session.id.clone();
            tabs = tabs.child(
                Button::new(("session-tab", index))
                    .ghost()
                    .small()
                    .selected(self.active_session_id.as_deref() == Some(session.id.as_str()))
                    .icon(match session.agent {
                        Agent::Claude => IconName::Bot,
                        Agent::Codex => IconName::SquareTerminal,
                    })
                    .label(session.title.clone())
                    .on_click(cx.listener(move |this, _, _, cx| this.select_session(id.clone(), cx))),
            );
        }
        tabs
    }

    fn restart_active(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(session_id) = self.active_session_id.clone() else {
            return;
        };
        self.run_session_request(session_id, false, cx);
    }

    fn open_active_details(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session().cloned() else {
            return;
        };
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let window_handle = self.window_handle;
        let (tx, rx) = async_channel::bounded(1);
        self.status_message = localized(
            matches!(self.preferences.locale, Locale::SimplifiedChinese),
            "Loading session details…",
            "正在加载会话详情…",
        )
        .into();
        self.runtime.spawn(async move {
            let result = api.session_details(&profile, credential.as_deref().map(|value| value.as_str()), &session.id).await;
            let _ = tx.send((session, result)).await;
        });
        cx.spawn(async move |this, cx| {
            if let Ok((session, result)) = rx.recv().await {
                let details = match result {
                    Ok(details) => details,
                    Err(error) => {
                        let _ = this.update(cx, |this, cx| {
                            this.status_message = error.to_string();
                            cx.notify();
                        });
                        return;
                    }
                };
                let _ = this.update(cx, |this, cx| {
                    this.status_message = localized(matches!(this.preferences.locale, Locale::SimplifiedChinese), "Connected", "已连接").into();
                    cx.notify();
                });
                let _ = window_handle.update(cx, move |_, window, cx| {
                    window.open_dialog(cx, move |dialog, _, _| {
                        dialog
                            .title("Session details")
                            .w(px(560.))
                            .child(session_details_content(&session, &details))
                            .footer(DialogFooter::new().child(DialogClose::new().child(Button::new("close-session-details").primary().label("Done"))))
                    });
                });
            }
        })
        .detach();
    }

    fn confirm_close_active(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session().cloned() else {
            return;
        };
        let view = cx.entity();
        let session_id = session.id.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let close_view = view.clone();
            let close_session_id = session_id.clone();
            dialog
                .title("Close managed session?")
                .child(format!("{} will be terminated in the daemon. This cannot be undone.", session.title))
                .footer(
                    DialogFooter::new()
                        .gap_2()
                        .child(DialogClose::new().child(Button::new("cancel-close-session").outline().label("Cancel")))
                        .child(Button::new("confirm-close-session").danger().label("Close session").on_click(move |_, window, cx| {
                            close_view.update(cx, |this, cx| this.run_session_request(close_session_id.clone(), true, cx));
                            window.close_dialog(cx);
                        })),
                )
        });
    }

    fn run_session_request(&mut self, session_id: String, close: bool, cx: &mut Context<Self>) {
        let profile = self.active_profile();
        let credential = self.credential(&profile);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.status_message = if close { "Closing session…".into() } else { "Restarting session…".into() };
        self.runtime.spawn(async move {
            let result = if close {
                api.close_session(&profile, credential.as_deref().map(|value| value.as_str()), &session_id).await
            } else {
                api.restart_session(&profile, credential.as_deref().map(|value| value.as_str()), &session_id).await
            };
            let _ = tx.send((session_id, result)).await;
        });
        cx.spawn(async move |this, cx| {
            if let Ok((session_id, result)) = rx.recv().await {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(()) if close => {
                        this.sessions.retain(|session| session.id != session_id);
                        this.terminals.remove(&session_id);
                        this.active_session_id = None;
                        this.status_message = localized(matches!(this.preferences.locale, Locale::SimplifiedChinese), "Connected", "已连接").into();
                        this.restore_or_select_session(cx);
                    }
                    Ok(()) => {
                        this.status_message = localized(matches!(this.preferences.locale, Locale::SimplifiedChinese), "Connected", "已连接").into();
                        cx.notify();
                    }
                    Err(error) => {
                        this.status_message = error.to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }
}

impl Render for DesktopWorkspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let profile = self.active_profile();
        let chinese = matches!(self.preferences.locale, Locale::SimplifiedChinese);
        let active_session = self.active_session().cloned();
        let collapsed = self.sidebar_collapsed;
        let terminal = self.active_session_id.as_ref().and_then(|id| self.terminals.get(id)).cloned();
        let has_terminal = terminal.is_some();
        let pending_identity = self.pending_identity.clone();
        let background = if self.preferences.material_enabled {
            cx.theme().background.opacity(1.0 - f32::from(self.preferences.material_transparency) / 100.0)
        } else {
            cx.theme().background
        };
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .text_color(cx.theme().foreground)
            .on_key_down(cx.listener(Self::on_workspace_key_down))
            .child(
                TitleBar::new().child(
                    div()
                        .h_flex()
                        .gap_2()
                        .items_center()
                        .child(Icon::new(IconName::GalleryVerticalEnd))
                        .child(div().font_semibold().child("AkironMux"))
                        .child(
                            Button::new("manage-backends")
                                .ghost()
                                .small()
                                .icon(IconName::Network)
                                .label(profile.name.clone())
                                .on_click(cx.listener(Self::open_backend_manager)),
                        )
                        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(self.status_message.clone())),
                ),
            )
            .when_some(pending_identity, |this, (_, observed)| {
                this.child(
                    div()
                        .h_flex()
                        .gap_3()
                        .items_center()
                        .px_4()
                        .py_2()
                        .bg(cx.theme().warning.opacity(0.15))
                        .child(Icon::new(IconName::TriangleAlert))
                        .child(div().flex_1().text_sm().child(if chinese {
                            format!("后端身份已变更为 {observed}。仅在确认服务器已被预期替换时信任。")
                        } else {
                            format!("Backend identity changed to {observed}. Confirm only if you expected the server replacement.")
                        }))
                        .child(
                            Button::new("cancel-identity")
                                .outline()
                                .label(localized(chinese, "Cancel", "取消"))
                                .on_click(cx.listener(Self::cancel_identity)),
                        )
                        .child(
                            Button::new("confirm-identity")
                                .warning()
                                .label(localized(chinese, "Trust new identity", "信任新身份"))
                                .on_click(cx.listener(Self::confirm_identity)),
                        ),
                )
            })
            .child(
                div()
                    .h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        Sidebar::new("akmux-sidebar")
                            .collapsible(SidebarCollapsible::Icon)
                            .collapsed(collapsed)
                            .w(px(self.preferences.sidebar_width))
                            .header(
                                SidebarHeader::new().child(div().h_flex().gap_2().items_center().child(Icon::new(IconName::Network)).when(!collapsed, |this| {
                                    this.child(
                                        div()
                                            .v_flex()
                                            .child(profile.name.clone())
                                            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(profile.address.clone())),
                                    )
                                })),
                            )
                            .child(SidebarGroup::new(localized(chinese, "Live sessions", "实时会话")).child(self.session_menu(cx)))
                            .child(SidebarGroup::new(localized(chinese, "Projects", "项目")).child(self.project_menu(cx)))
                            .child(SidebarGroup::new(localized(chinese, "General", "通用")).child(self.directory_menu(false, cx)))
                            .child(SidebarGroup::new(localized(chinese, "Other", "其他")).child(self.directory_menu(true, cx)))
                            .footer(
                                SidebarFooter::new()
                                    .child(
                                        Button::new("manage-projects")
                                            .ghost()
                                            .icon(IconName::Folder)
                                            .label(localized(chinese, "Manage projects", "管理项目"))
                                            .on_click(cx.listener(Self::open_project_manager)),
                                    )
                                    .child(
                                        Button::new("toggle-sidebar")
                                            .ghost()
                                            .icon(if collapsed { IconName::PanelLeftOpen } else { IconName::PanelLeftClose })
                                            .label(if collapsed {
                                                localized(chinese, "Expand sidebar", "展开侧栏")
                                            } else {
                                                localized(chinese, "Collapse sidebar", "收起侧栏")
                                            })
                                            .on_click(cx.listener(Self::toggle_sidebar)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(
                                div()
                                    .h_flex()
                                    .h_12()
                                    .px_2()
                                    .gap_2()
                                    .items_center()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .child(self.session_tabs(cx))
                                    .child(
                                        Button::new("search-history")
                                            .ghost()
                                            .icon(IconName::Search)
                                            .label(localized(chinese, "Search", "搜索"))
                                            .on_click(cx.listener(Self::open_search)),
                                    )
                                    .when(active_session.is_some(), |this| {
                                        this.child(
                                            Button::new("session-details")
                                                .ghost()
                                                .icon(IconName::Info)
                                                .label(localized(chinese, "Details", "详情"))
                                                .on_click(cx.listener(Self::open_active_details)),
                                        )
                                        .child(
                                            Button::new("restart-session")
                                                .ghost()
                                                .icon(IconName::Redo)
                                                .label(localized(chinese, "Restart", "重启"))
                                                .on_click(cx.listener(Self::restart_active)),
                                        )
                                        .child(
                                            Button::new("close-session")
                                                .danger()
                                                .icon(IconName::Close)
                                                .label(localized(chinese, "Close", "关闭"))
                                                .on_click(cx.listener(Self::confirm_close_active)),
                                        )
                                    })
                                    .child(
                                        Button::new("refresh")
                                            .ghost()
                                            .icon(IconName::Redo)
                                            .label(localized(chinese, "Refresh", "刷新"))
                                            .on_click(cx.listener(|this, _, _, cx| this.start_polling(cx))),
                                    )
                                    .child(
                                        Button::new("settings")
                                            .ghost()
                                            .icon(IconName::Settings)
                                            .label(localized(chinese, "Settings", "设置"))
                                            .on_click(cx.listener(Self::open_settings)),
                                    )
                                    .child(
                                        Button::new("new-session")
                                            .primary()
                                            .icon(IconName::Plus)
                                            .label(localized(chinese, "New session", "新建会话"))
                                            .on_click(cx.listener(Self::open_new_session)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_hidden()
                                    .when_some(terminal, |this, terminal| this.child(terminal))
                                    .when(!has_terminal, |this| {
                                        this.v_flex()
                                            .items_center()
                                            .justify_center()
                                            .gap_2()
                                            .child(Icon::new(IconName::SquareTerminal).size_8())
                                            .child(div().font_semibold().child(localized(chinese, "Select a live session", "选择一个实时会话")))
                                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(localized(
                                                chinese,
                                                "AkironMux keeps the PTY in the daemon when this window closes.",
                                                "窗口关闭后，AkironMux 仍会在守护进程中保留 PTY。",
                                            )))
                                    }),
                            )
                            .child(
                                div()
                                    .h_7()
                                    .px_3()
                                    .h_flex()
                                    .items_center()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if chinese {
                                        format!("{} 个会话", self.sessions.len())
                                    } else {
                                        format!("{} session{}", self.sessions.len(), if self.sessions.len() == 1 { "" } else { "s" })
                                    })
                                    .child(div().flex_1())
                                    .child(self.status_message.clone()),
                            ),
                    ),
            )
            .when(self.directory_picker.is_some(), |this| this.child(self.directory_picker_overlay(cx)))
    }
}

fn config_directory() -> std::path::PathBuf {
    dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("dev.akiron.mux")
}

fn localized(chinese: bool, english: &'static str, simplified_chinese: &'static str) -> &'static str {
    if chinese { simplified_chinese } else { english }
}

fn apply_window_material(window: &Window, enabled: bool) {
    window.set_background_appearance(if enabled {
        if cfg!(target_os = "windows") {
            WindowBackgroundAppearance::MicaBackdrop
        } else {
            WindowBackgroundAppearance::Blurred
        }
    } else {
        WindowBackgroundAppearance::Opaque
    });
}

fn setting_group(title: &'static str, content: impl IntoElement) -> impl IntoElement {
    v_flex().gap_2().child(div().text_sm().font_semibold().child(title)).child(content)
}

fn session_details_content(session: &SessionInfo, details: &SessionDetails) -> impl IntoElement {
    let provider = details.provider_name.clone().or_else(|| details.provider_id.clone()).unwrap_or_else(|| "—".into());
    let model = details.model.clone().unwrap_or_else(|| "—".into());
    let native_id = details.native_session_id.clone().unwrap_or_else(|| "—".into());
    v_flex()
        .gap_2()
        .child(detail_row("Managed session", session.id.clone()))
        .child(detail_row("Native session", native_id))
        .child(detail_row("Agent", agent_name(details.agent)))
        .child(detail_row("Provider", provider))
        .child(detail_row("Model", model))
        .child(detail_row("Working directory", session.cwd.clone()))
        .child(detail_row("Messages", details.message_count.to_string()))
        .child(detail_row("Prompt tokens", details.prompt_tokens.to_string()))
        .child(detail_row("Completion tokens", details.completion_tokens.to_string()))
        .child(detail_row("Cache read tokens", details.cache_read_tokens.to_string()))
        .child(detail_row("Cache creation tokens", details.cache_creation_tokens.to_string()))
}

fn detail_row(label: &'static str, value: impl Into<SharedString>) -> impl IntoElement {
    h_flex()
        .gap_4()
        .items_start()
        .child(div().w(px(150.)).text_sm().text_color(rgb(0x7F8C98)).child(label))
        .child(div().flex_1().min_w_0().text_sm().child(value.into()))
}

fn agent_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "Claude Code",
        Agent::Codex => "Codex",
    }
}
