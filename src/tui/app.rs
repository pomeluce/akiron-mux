use std::rc::Rc;
use std::sync::mpsc;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{backend::CrosstermBackend, Frame, Terminal};

use crate::agent::AgentKind;
use crate::core::config::ConfigManager;
use crate::core::models::AppType;
use crate::core::native_history::NativeHistoryIngestion;

use super::tabs::{history::HistoryTab, providers::ProvidersTab, settings::SettingsTab, usage::UsageTab, Tab, TabContent};
use super::theme;

pub struct App {
    pub mgr: Rc<ConfigManager>,
    pub active_tab: Tab,
    pub providers_tab: ProvidersTab,
    pub usage_tab: UsageTab,
    pub history_tab: HistoryTab,
    pub settings_tab: SettingsTab,
    pub current_app: AppType,
    pub should_quit: bool,
    /// cached proxy_mode to avoid per-frame DB query
    proxy_mode: bool,
    /// Near-real-time polling channel — receives true when session files change
    poll_rx: Option<mpsc::Receiver<bool>>,
}

impl App {
    pub fn new(db_path: &std::path::Path, defaults_path: Option<&std::path::Path>) -> anyhow::Result<Self> {
        let mgr = Rc::new(ConfigManager::new(db_path, defaults_path)?);
        if let Err(e) = NativeHistoryIngestion::new(mgr.db()).refresh_sessions(AgentKind::Codex, |_| {}) {
            tracing::warn!("Failed to import Codex sessions: {}", e);
        }
        let current_app = mgr.get_setting("active_app").and_then(|value| value.parse().ok()).unwrap_or_default();
        let proxy_mode = mgr.get_setting("proxy_mode").map(|v| v == "true").unwrap_or(false);
        let providers_tab = ProvidersTab::new(mgr.clone(), current_app);
        let usage_tab = UsageTab::new(mgr.clone(), current_app);
        let history_tab = HistoryTab::new(mgr.clone(), current_app);
        let settings_tab = SettingsTab::new(mgr.clone());
        let poll_rx = Some(super::file_watcher::spawn_polling_thread(1));

        Ok(App {
            mgr,
            active_tab: Tab::Providers,
            providers_tab,
            usage_tab,
            history_tab,
            settings_tab,
            current_app,
            should_quit: false,
            proxy_mode,
            poll_rx,
        })
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        let mut terminal = ratatui::init();
        let result = self.event_loop(&mut terminal);
        ratatui::restore();
        self.usage_tab.shutdown();
        result
    }

    fn event_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> anyhow::Result<()> {
        while !self.should_quit {
            self.usage_tab.poll_scan_events();
            self.poll_file_changes();

            // Refresh proxy_mode once per tick (cheap DB read, avoids per-frame query)
            self.proxy_mode = self.mgr.get_setting("proxy_mode").map(|v| v == "true").unwrap_or(false);

            terminal.draw(|f| self.render(f))?;

            if event::poll(std::time::Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(key.code);
                    }
                }
            }
            if self.history_tab.needs_terminal_reinit {
                ratatui::restore();
                if let Some(ref project) = self.history_tab.launch_project.take() {
                    let sid = self.history_tab.launch_session_id.take().unwrap_or_default();
                    println!("\n  Launching {} session {} in {}\n", self.current_app.display_name(), sid, project);
                    let mut cmd = if self.current_app == AppType::Codex {
                        let mut command = std::process::Command::new("codex");
                        if !sid.is_empty() {
                            command.args(["resume", &sid]);
                        }
                        command
                    } else {
                        let mut command = std::process::Command::new("claude");
                        if !sid.is_empty() {
                            command.args(["--resume", &sid]);
                        }
                        command
                    };
                    cmd.current_dir(project);
                    if let Err(e) = cmd.status() {
                        eprintln!("Failed to launch Claude: {}", e);
                    }
                    print!("\n  Returning to AkironMux...\n");
                }
                *terminal = ratatui::init();
                self.history_tab.needs_terminal_reinit = false;
            }
        }
        Ok(())
    }

    fn poll_file_changes(&mut self) {
        if let Some(rx) = &self.poll_rx {
            match rx.try_recv() {
                Ok(true) => {
                    tracing::info!("File watcher: changes detected, running incremental imports");
                    let ingestion = NativeHistoryIngestion::new(self.mgr.db());
                    for result in [ingestion.refresh_sessions(AgentKind::Claude, |_| {}), ingestion.refresh_sessions(AgentKind::Codex, |_| {})] {
                        if let Err(e) = result {
                            tracing::warn!("Polling session import failed: {}", e);
                        }
                    }
                    self.history_tab.reload_current();
                    if !self.usage_tab.is_scanning() {
                        self.usage_tab.trigger_incremental_scan();
                    }
                }
                Ok(false) => {}
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.poll_rx = None;
                }
            }
        }
    }

    fn handle_key(&mut self, code: KeyCode) {
        let handled = match self.active_tab {
            Tab::Providers => self.providers_tab.handle_key(code),
            Tab::Usage => self.usage_tab.handle_key(code),
            Tab::History => self.history_tab.handle_key(code),
            Tab::Settings => self.settings_tab.handle_key(code),
        };
        if handled {
            return;
        }

        match code {
            KeyCode::Tab => {
                self.next_tab();
            }
            KeyCode::BackTab => {
                self.prev_tab();
            }
            KeyCode::Char(' ') => self.toggle_app(),
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            _ => {}
        }
    }

    fn toggle_app(&mut self) {
        self.current_app = self.current_app.toggle();
        self.providers_tab.switch_app(self.current_app);
        self.usage_tab.set_app(self.current_app);
        self.history_tab.set_app(self.current_app);
        if let Err(e) = self.mgr.set_setting("active_app", self.current_app.as_str()) {
            tracing::warn!("Failed to persist active app: {}", e);
        }
    }

    fn next_tab(&mut self) {
        self.active_tab = match self.active_tab {
            Tab::Providers => Tab::Usage,
            Tab::Usage => Tab::History,
            Tab::History => Tab::Settings,
            Tab::Settings => Tab::Providers,
        };
    }

    fn prev_tab(&mut self) {
        self.active_tab = match self.active_tab {
            Tab::Providers => Tab::Settings,
            Tab::Settings => Tab::History,
            Tab::Usage => Tab::Providers,
            Tab::History => Tab::Usage,
        };
    }

    fn render(&mut self, f: &mut Frame) {
        use super::widgets::app_bar::render_app_bar;
        use super::widgets::shared::{render_shortcut_bar, render_status_bar, shortcut_line_count};
        use super::widgets::sidebar::render_sidebar;
        use ratatui::layout::{Constraint, Direction, Layout};

        let area = f.area();

        // Calculate shortcut bar height for the active tab (width = main area, ~sidebar 14 cols)
        let sidebar_width = if area.width < 76 { 13 } else { 16 };
        let mut groups = match self.active_tab {
            Tab::Providers => self.providers_tab.shortcut_groups(),
            Tab::Usage => self.usage_tab.shortcut_groups(),
            Tab::History => self.history_tab.shortcut_groups(),
            Tab::Settings => self.settings_tab.shortcut_groups(),
        };
        groups.push(vec![(" Space ".into(), theme::current().comment), ("Claude/Codex".into(), theme::current().comment)]);
        let sc_lines = shortcut_line_count(area.width, &groups);

        // Global bars span the full terminal; the sidebar only occupies the middle row.
        let [app_bar_area, body_area, status_area, sc_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3), Constraint::Length(1), Constraint::Length(2 + sc_lines as u16)])
            .areas(area);

        let [sidebar_area, content_area] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(sidebar_width), Constraint::Min(20)])
            .areas(body_area);

        let is_proxy = self.proxy_mode;
        render_sidebar(f, sidebar_area, self.active_tab);
        let active_context = self.providers_tab.active_context();
        render_app_bar(f, app_bar_area, self.current_app, is_proxy, &active_context);

        let status = match self.active_tab {
            Tab::Providers => self.providers_tab.status_text(),
            Tab::Usage => self.usage_tab.status_text(),
            Tab::History => self.history_tab.status_text(),
            Tab::Settings => self.settings_tab.status_text(),
        };
        render_status_bar(f, status_area, &status);

        render_shortcut_bar(f, sc_area, &groups);

        // Pages render last so their modal overlays cannot be overwritten by global bars.
        match self.active_tab {
            Tab::Providers => self.providers_tab.render(f, content_area),
            Tab::Usage => self.usage_tab.render(f, content_area),
            Tab::History => self.history_tab.render(f, content_area),
            Tab::Settings => self.settings_tab.render(f, content_area),
        }
    }
}
