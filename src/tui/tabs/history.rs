use crate::tui::lang;
use std::sync::Arc;

use super::super::theme;
use super::super::widgets::session_detail::{render_empty_detail, render_session_detail};
use super::super::widgets::shared::{format_size, relative_time, render_confirm_popup as shared_confirm, render_search_box as shared_search, truncate};
use super::TabContent;
use crate::core::config::ConfigManager;
use crate::core::models::AppType;
use crate::db::sessions::SessionRecord;
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState},
    Frame,
};

#[derive(Clone, Copy, PartialEq)]
pub enum ConfirmAction {
    Open,
    Delete,
}

pub struct HistoryTab {
    pub all_sessions: Vec<SessionRecord>,
    pub sessions: Vec<SessionRecord>,
    pub state: ListState,
    pub search_query: String,
    pub is_searching: bool,
    pub confirm_action: Option<ConfirmAction>,
    pub needs_terminal_reinit: bool,
    pub launch_project: Option<String>,
    pub launch_session_id: Option<String>,
    confirm_button: usize, // 0=Confirm, 1=Cancel
    /// Cached active provider/profile for session detail panel (avoid per-frame DB reads)
    cached_prov_name: String,
    cached_prof_name: String,
    /// Cached token count for the selected session
    cached_tokens: Option<(i64, i64)>,
    cached_tokens_sid: String,
    app: AppType,
    mgr: Arc<ConfigManager>,
}

impl HistoryTab {
    pub fn new(mgr: Arc<ConfigManager>, app: AppType) -> Self {
        // Session import is handled before TUI launch (in main.rs with progress bar).
        // Just load whatever is already in the DB.
        let all = mgr
            .db()
            .query_sessions(app.as_str(), None, None, 200)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.size_bytes > 0)
            .collect::<Vec<_>>();
        let sessions = all.clone();
        let mut state = ListState::default();
        if !sessions.is_empty() {
            state.select(Some(0));
        }
        HistoryTab {
            all_sessions: all,
            sessions,
            state,
            search_query: String::new(),
            is_searching: false,
            confirm_action: None,
            needs_terminal_reinit: false,
            launch_project: None,
            launch_session_id: None,
            confirm_button: 0,
            cached_prov_name: String::new(),
            cached_prof_name: String::new(),
            cached_tokens: None,
            cached_tokens_sid: String::new(),
            app,
            mgr,
        }
    }

    pub fn set_app(&mut self, app: AppType) {
        self.app = app;
        self.search_query.clear();
        self.is_searching = false;
        self.confirm_action = None;
        self.cached_tokens = None;
        self.cached_tokens_sid.clear();
        self.cached_prov_name.clear();
        self.cached_prof_name.clear();
        let import_result = match app {
            AppType::Claude => crate::core::import::import_claude_sessions(self.mgr.db()),
            AppType::Codex => crate::core::import::import_codex_sessions(self.mgr.db()),
        };
        if let Err(e) = import_result {
            tracing::warn!("Failed to import {} sessions: {}", app.as_str(), e);
        }
        self.reload_current();
        self.state.select(if self.sessions.is_empty() { None } else { Some(0) });
    }

    pub fn reload_current(&mut self) {
        self.all_sessions = self.mgr.db()
            .query_sessions(self.app.as_str(), None, None, 200)
            .unwrap_or_default()
            .into_iter()
            .filter(|session| session.size_bytes > 0)
            .collect();
        self.refresh();
    }

    pub fn refresh(&mut self) {
        let query = self.search_query.trim().to_lowercase();
        let tokens: Vec<&str> = query.split_whitespace().collect();
        self.sessions = self
            .all_sessions
            .iter()
            .filter(|s| {
                if tokens.is_empty() {
                    return true;
                }
                tokens.iter().all(|t| s.search_text.contains(t))
            })
            .cloned()
            .collect();
        if self.state.selected().unwrap_or(0) >= self.sessions.len() {
            self.state.select(if self.sessions.is_empty() { None } else { Some(0) });
        }
    }

    fn delete_selected(&mut self) {
        if let Some(idx) = self.state.selected() {
            if idx < self.sessions.len() {
                if let Some(session) = self.sessions.get(idx) {
                    if let Err(e) = self.mgr.db().delete_session(&session.id, self.app.as_str()) {
                        tracing::warn!("Failed to delete session from database: {}", e);
                    }
                    self.all_sessions.retain(|s| s.id != session.id);
                }
                self.sessions.remove(idx);
                let len = self.sessions.len();
                if len == 0 {
                    self.state.select(None);
                } else if idx >= len {
                    self.state.select(Some(len - 1));
                } else {
                    self.state.select(Some(idx));
                }
            }
        }
    }

    /// Signal App to launch claude (terminal suspend/restore handled by event loop)
    fn open_session(&mut self) {
        if let Some(idx) = self.state.selected() {
            if let Some(s) = self.sessions.get(idx) {
                self.needs_terminal_reinit = true;
                self.launch_project = Some(s.project_path.clone());
                self.launch_session_id = Some(s.id.clone());
            }
        }
    }
}

impl TabContent for HistoryTab {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // Left panel: search box + session list
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(main[0]);

        // Right panel: detail preview
        let right_chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(3)]).split(main[1]);

        // Search box
        self.render_search_box(f, left_chunks[0]);

        // Session list — 2-line items
        let items: Vec<ListItem> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let is_selected = self.state.selected() == Some(i);

                let raw = s.title.as_deref().unwrap_or(&s.id);
                let is_uuid = raw.len() >= 32 && raw.chars().filter(|c| *c == '-').count() >= 4;
                let title = if is_uuid {
                    std::path::Path::new(&s.project_path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| raw.to_string())
                } else {
                    raw.to_string()
                };
                let title = truncate(&title, 50);

                let project = std::path::Path::new(&s.project_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                let date = if s.file_mtime.is_empty() {
                    relative_time(&s.start_time)
                } else {
                    relative_time(&s.file_mtime)
                };
                let size = format_size(s.size_bytes);

                let arrow = if is_selected { "❯ " } else { "  " };
                let title_color = if is_selected { theme::current().cyan } else { theme::current().fg };

                ListItem::new(vec![
                    Line::from(Span::styled(format!("{}{}", arrow, title), Style::default().fg(title_color))),
                    Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(date, Style::default().fg(theme::current().comment)),
                        Span::styled(" · ", Style::default().fg(theme::current().dim)),
                        Span::styled(project, Style::default().fg(theme::current().comment)),
                        Span::styled(" · ", Style::default().fg(theme::current().dim)),
                        Span::styled(size, Style::default().fg(theme::current().comment)),
                    ]),
                    // Spacing between items
                    Line::from(""),
                ])
            })
            .collect();

        let count = items.len();
        let list = List::new(items)
            .block(
                Block::bordered()
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .title(format!("{} ({})", lang::current().sessions_title, count))
                    .border_style(Style::default().fg(theme::current().dim)),
            )
            .highlight_style(Style::default());

        f.render_stateful_widget(list, left_chunks[1], &mut self.state);

        // Right panel: detail preview
        if let Some(idx) = self.state.selected() {
            if let Some(s) = self.sessions.get(idx) {
                // Cache tokens and provider/profile names — reload only on selection change
                if self.cached_tokens_sid != s.id {
                    self.cached_tokens = self.mgr.db().query_session_tokens(&s.id).ok();
                    self.cached_tokens_sid = s.id.clone();
                    let prov = self.mgr.get_setting(self.app.active_provider_key()).unwrap_or_default();
                    let prof = if self.app == AppType::Claude {
                        self.mgr.get_setting("active_profile").unwrap_or_default()
                    } else {
                        String::new()
                    };
                    self.cached_prov_name = prov;
                    self.cached_prof_name = prof;
                }
                let prov_name = if self.cached_prov_name.is_empty() { None } else { Some(self.cached_prov_name.as_str()) };
                let prof_name = if self.cached_prof_name.is_empty() { None } else { Some(self.cached_prof_name.as_str()) };
                render_session_detail(f, right_chunks[0], s, self.cached_tokens, prov_name, prof_name, self.app);
            } else {
                render_empty_detail(f, right_chunks[0], "No session selected");
            }
        } else {
            render_empty_detail(f, right_chunks[0], lang::current().no_sessions);
        }

        // Shortcut bar under detail preview

        // Confirmation popup
        if self.confirm_action.is_some() {
            self.render_confirm_popup(f, area);
        }
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        // Confirm popup mode (open or delete)
        if self.confirm_action.is_some() {
            match code {
                KeyCode::Tab | KeyCode::Right | KeyCode::Char('j') | KeyCode::Char('l') => {
                    self.confirm_button = (self.confirm_button + 1) % 2;
                }
                KeyCode::BackTab | KeyCode::Left | KeyCode::Char('k') | KeyCode::Char('h') => {
                    self.confirm_button = if self.confirm_button == 0 { 1 } else { 0 };
                }
                KeyCode::Enter => {
                    if self.confirm_button == 0 {
                        match self.confirm_action {
                            Some(ConfirmAction::Open) => self.open_session(),
                            Some(ConfirmAction::Delete) => self.delete_selected(),
                            _ => {}
                        }
                    }
                    self.confirm_action = None;
                    self.confirm_button = 0;
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.confirm_action = None;
                    self.confirm_button = 0;
                }
                _ => {}
            }
            return true;
        }

        // Search mode
        if self.is_searching {
            match code {
                KeyCode::Esc => {
                    self.is_searching = false;
                    self.search_query.clear();
                    self.refresh();
                }
                KeyCode::Enter => {
                    self.is_searching = false;
                    if !self.sessions.is_empty() {
                        self.state.select(Some(0));
                    }
                }
                KeyCode::Backspace | KeyCode::Delete => {
                    self.search_query.pop();
                    self.refresh();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.refresh();
                }
                _ => {}
            }
            return true;
        }

        // List mode
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                let len = self.sessions.len();
                if len == 0 {
                    return true;
                }
                let i = self.state.selected().unwrap_or(0);
                self.state.select(Some(if i + 1 < len { i + 1 } else { 0 }));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let len = self.sessions.len();
                if len == 0 {
                    return true;
                }
                let i = self.state.selected().unwrap_or(0);
                self.state.select(Some(if i > 0 { i - 1 } else { len - 1 }));
            }
            KeyCode::Enter => {
                self.confirm_action = Some(ConfirmAction::Open);
                self.confirm_button = 0;
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.confirm_action = Some(ConfirmAction::Delete);
                self.confirm_button = 0;
            }
            KeyCode::Char('/') => {
                self.is_searching = true;
            }
            _ => {
                return false;
            }
        }
        true
    }

    fn shortcut_groups(&self) -> Vec<Vec<(String, Color)>> {
        vec![
            vec![(" J/K ".into(), theme::current().comment), (lang::current().sc_nav.into(), theme::current().comment)],
            vec![(" / ".into(), theme::current().comment), (lang::current().sc_search.into(), theme::current().comment)],
            vec![(" ⏎  ".into(), theme::current().comment), (lang::current().sc_open.into(), theme::current().comment)],
            vec![(" D ".into(), theme::current().comment), (lang::current().sc_delete.into(), theme::current().comment)],
            vec![(" Q ".into(), theme::current().comment), (lang::current().sc_quit.into(), theme::current().comment)],
        ]
    }

    fn shortcut_lines(&self, available_width: u16) -> usize {
        let group_widths = [8usize, 9, 12, 7, 7];
        let sep = 2usize;
        let w = available_width.saturating_sub(2).max(10) as usize;
        let mut lines = 1usize;
        let mut cur = 0usize;
        for gw in &group_widths {
            if cur + gw > w && cur > 0 {
                lines += 1;
                cur = 0;
            }
            if cur > 0 {
                cur += sep;
            }
            cur += gw;
        }
        lines
    }
}

impl HistoryTab {
    fn render_confirm_popup(&self, f: &mut Frame, area: Rect) {
        let is_delete = self.confirm_action == Some(ConfirmAction::Delete);
        let (title, msg, c) = if is_delete {
            (lang::current().confirm_delete_title, lang::current().confirm_delete_session_msg, theme::current().red)
        } else {
            (lang::current().confirm_open_title, lang::current().confirm_open_msg, theme::current().cyan)
        };
        shared_confirm(f, area, title, msg, lang::current().confirm_confirm, lang::current().confirm_cancel, c, self.confirm_button);
    }

    fn render_search_box(&self, f: &mut Frame, area: Rect) {
        shared_search(f, area, &self.search_query, self.is_searching);
    }
}
