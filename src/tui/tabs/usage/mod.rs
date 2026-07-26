pub mod chart;
use crate::tui::lang;
pub mod scan;

use super::super::theme;
use super::super::widgets::shared::{format_tokens, render_search_box as shared_search};
use super::TabContent;
use crate::core::config::ConfigManager;
use crate::core::models::AppType;
use crate::db::usage::{ScanContext, ScanEvent, UsageSummary};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::sync::mpsc;
use std::sync::Arc;

/// Background scan state, updated by poll_scan_events()
enum ScanState {
    Idle,
    Scanning { files_done: usize, files_total: usize, records: usize },
}

pub struct UsageTab {
    mgr: Arc<ConfigManager>,
    pub summaries: Vec<UsageSummary>,
    pub state: ListState,
    pub selected_index: usize,
    pub range: String,
    pub search_query: String,
    pub is_searching: bool,
    chart_scroll: usize,
    app_type: String,
    scan_app: String,
    /// Cached daily usage to avoid per-frame DB queries
    cached_daily: Option<(String, Vec<(String, i64, i64, i64, i64)>)>,
    /// Background scan receiver + state
    scan_rx: Option<mpsc::Receiver<ScanEvent>>,
    scan_state: ScanState,
    /// Handle for graceful shutdown of the background scan thread
    scan_handle: Option<std::thread::JoinHandle<()>>,
    /// A file change arrived while a scan was running.
    rescan_pending: bool,
}

impl UsageTab {
    pub fn new(mgr: Arc<ConfigManager>, app: AppType) -> Self {
        let scan_state;
        let scan_rx;
        let scan_handle;
        let app_type = app.as_str().to_string();

        // Check if this is first launch (no usage data yet)
        let is_first_launch = {
            let db = mgr.db();
            let count: i64 = db.conn().query_row(
                "SELECT COUNT(*) FROM usage_logs WHERE app_type = ?1",
                [&app_type],
                |r| r.get(0),
            ).unwrap_or(0);
            count == 0
        };

        // Prepare scan context on main thread (DB access only, fast) then spawn background parser
        {
            let (tx, rx) = mpsc::channel();
            let ctx = match mgr.db().prepare_scan_context(&app_type) {
                Ok(c) => {
                    tracing::info!("Scan prep: {} known msg IDs, {} files in index", c.known_msg_ids.len(), c.file_index.len());
                    c
                }
                Err(e) => {
                    tracing::error!("Failed to prepare scan context: {}", e);
                    ScanContext {
                        known_msg_ids: Vec::new(),
                        file_index: std::collections::HashMap::new(),
                    }
                }
            };
            // Always spawn background thread — it does its own file collection
            let scan_target = app_type.clone();
            let handle = std::thread::spawn(move || {
                crate::core::import::parse_files_in_background(scan_target, ctx, 10, tx);
            });
            scan_rx = Some(rx);
            scan_handle = Some(handle);
            if is_first_launch {
                scan_state = ScanState::Scanning {
                    files_done: 0,
                    files_total: 0,
                    records: 0,
                };
            } else {
                scan_state = ScanState::Idle;
            }
        }

        let summaries = mgr.db().query_usage(&app_type, "all").unwrap_or_default();
        let mut state = ListState::default();
        if !summaries.is_empty() {
            state.select(Some(0));
        }
        UsageTab {
            mgr,
            summaries,
            state,
            selected_index: 0,
            range: "all".into(),
            search_query: String::new(),
            is_searching: false,
            chart_scroll: 0,
            scan_app: app_type.clone(),
            app_type,
            cached_daily: None,
            scan_rx,
            scan_state,
            scan_handle,
            rescan_pending: false,
        }
    }

    pub fn set_app(&mut self, app: AppType) {
        self.app_type = app.as_str().to_string();
        self.cached_daily = None;
        self.selected_index = 0;
        self.summaries = self.mgr.db().query_usage(&self.app_type, &self.range).unwrap_or_default();
        self.state.select(if self.summaries.is_empty() { None } else { Some(0) });
        if self.scan_handle.is_none() {
            self.trigger_incremental_scan();
        } else {
            self.rescan_pending = true;
        }
    }

    /// Check if a background scan is currently running
    pub fn is_scanning(&self) -> bool {
        matches!(self.scan_state, ScanState::Scanning { .. })
    }

    /// Trigger an incremental scan (called by file watcher when changes detected)
    pub fn trigger_incremental_scan(&mut self) {
        if self.scan_handle.is_some() {
            self.rescan_pending = true;
            return;
        }
        let scan_app = self.app_type.clone();
        let ctx = match self.mgr.db().prepare_scan_context(&scan_app) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to prepare incremental scan: {}", e);
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        let worker_app = scan_app.clone();
        let handle = std::thread::spawn(move || {
            crate::core::import::parse_files_in_background(worker_app, ctx, 10, tx);
        });
        self.scan_rx = Some(rx);
        self.scan_handle = Some(handle);
        self.scan_app = scan_app;
        self.rescan_pending = false;
        // Keep Idle — silent background scan, no progress bar in UI
    }

    /// Gracefully wait for background scan thread to finish.
    pub fn shutdown(&mut self) {
        if let Some(handle) = self.scan_handle.take() {
            self.scan_rx = None;
            let _ = handle.join();
        }
    }

    fn token_total(s: &UsageSummary) -> i64 {
        s.total_prompt + s.total_completion + s.total_cache_read + s.total_cache_create
    }
    fn total_tokens(&self) -> i64 {
        self.summaries.iter().map(|s| Self::token_total(s)).sum()
    }
    fn max_tokens(&self) -> i64 {
        self.summaries.iter().map(|s| Self::token_total(s)).max().unwrap_or(1)
    }

    /// Called every event-loop tick — drain ALL pending events at once to avoid
    /// batching delays (one-per-tick would take N×100ms for N files).
    pub fn poll_scan_events(&mut self) {
        let rx = match &self.scan_rx {
            Some(rx) => rx,
            None => return,
        };

        let mut done = false;
        loop {
            match rx.try_recv() {
                Ok(ScanEvent::Batch { app_type, sid, file_path, records }) => {
                    if !records.is_empty() {
                        if let Err(e) = self.mgr.db().insert_usage_batch(&app_type, &sid, &file_path, &records) {
                            tracing::error!("Failed to insert usage batch: {}", e);
                        }
                    }
                }
                Ok(ScanEvent::Progress { files_done, files_total, records }) => {
                    if matches!(self.scan_state, ScanState::Scanning { .. }) {
                        self.scan_state = ScanState::Scanning { files_done, files_total, records };
                    }
                }
                Ok(ScanEvent::Done {}) => {
                    done = true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.scan_state = ScanState::Idle;
                    self.scan_rx = None;
                    self.scan_handle = None;
                    return;
                }
            }
        }

        if done {
            let completed_app = self.scan_app.clone();
            tracing::info!("Usage scan complete");
            self.scan_state = ScanState::Idle;
            self.scan_rx = None;
            if let Some(h) = self.scan_handle.take() {
                let _ = h.join();
            }
            self.cached_daily = None;
            self.summaries = self.mgr.db().query_usage(&self.app_type, &self.range).unwrap_or_default();
            if !self.summaries.is_empty() && self.state.selected().is_none() {
                self.state.select(Some(0));
            }
            if self.rescan_pending || completed_app != self.app_type {
                self.trigger_incremental_scan();
            }
        }
    }
}

impl TabContent for UsageTab {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        let left = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(4), Constraint::Min(3)])
            .split(main[0]);

        // Search box
        self.render_search_box(f, left[0]);
        // Summary cards
        self.render_summary_cards(f, left[1]);
        // Profile ranking
        self.render_profile_list(f, left[2]);

        // Right: daily chart (or scan progress)
        let right = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(3)]).split(main[1]);

        match &self.scan_state {
            ScanState::Scanning { files_done, files_total, records } => {
                self.render_scan_progress(f, right[0], *files_done, *files_total, *records);
            }
            ScanState::Idle => {
                self.render_daily_chart(f, right[0]);
            }
        }
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        if self.is_searching {
            match code {
                KeyCode::Esc => {
                    self.is_searching = false;
                    self.search_query.clear();
                }
                KeyCode::Enter => {
                    self.is_searching = false;
                }
                KeyCode::Backspace | KeyCode::Delete => {
                    self.search_query.pop();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                }
                _ => {}
            }
            return true;
        }
        match code {
            KeyCode::Tab | KeyCode::BackTab => return false,
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selected_index + 1 < self.summaries.len() {
                    self.selected_index += 1;
                    self.state.select(Some(self.selected_index));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                    self.state.select(Some(self.selected_index));
                }
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.range = match self.range.as_str() {
                    "day" => "week",
                    "week" => "month",
                    _ => "day",
                }
                .into();
                self.cached_daily = None;
                self.summaries = self.mgr.db().query_usage(&self.app_type, &self.range).unwrap_or_default();
            }
            KeyCode::Char('/') => {
                self.is_searching = true;
            }
            KeyCode::PageUp => {
                self.chart_scroll = self.chart_scroll.saturating_sub(5);
            }
            KeyCode::PageDown => {
                self.chart_scroll = self.chart_scroll.saturating_add(5);
            }
            _ => return false,
        }
        true
    }

    fn shortcut_groups(&self) -> Vec<Vec<(String, Color)>> {
        vec![
            vec![(" J/K ".into(), theme::current().comment), (lang::current().sc_nav.into(), theme::current().comment)],
            vec![(" / ".into(), theme::current().comment), (lang::current().sc_search.into(), theme::current().comment)],
            vec![(" T ".into(), theme::current().comment), (lang::current().sc_toggle.into(), theme::current().comment)],
            vec![(" PgUp/Dn ".into(), theme::current().comment), (lang::current().sc_scroll.into(), theme::current().comment)],
            vec![(" Q ".into(), theme::current().comment), (lang::current().sc_quit.into(), theme::current().comment)],
        ]
    }

    fn shortcut_lines(&self, available_width: u16) -> usize {
        let widths = [9usize, 10, 10, 17, 8];
        let sep = 2usize;
        let w = available_width.saturating_sub(2).max(10) as usize;
        let mut lines = 1usize;
        let mut cur = 0usize;
        for gw in &widths {
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

impl UsageTab {
    fn render_search_box(&self, f: &mut Frame, area: Rect) {
        shared_search(f, area, &self.search_query, self.is_searching);
    }

    fn get_daily_cached(&mut self, model: &str) -> Vec<(String, i64, i64, i64, i64)> {
        let key = format!("{}|{}", self.app_type, model);
        if let Some((ref k, ref data)) = self.cached_daily {
            if k == &key { return data.clone(); }
        }
        let data = self.mgr.db().query_daily_usage(&self.app_type, model).unwrap_or_default();
        self.cached_daily = Some((key, data.clone()));
        data
    }

    fn render_summary_cards(&mut self, f: &mut Frame, area: Rect) {
        let cards = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Ratio(1, 4); 4]).split(area);

        let model_name = self.summaries.get(self.selected_index).map(|s| s.model.clone());
        let daily = model_name.as_deref().map(|m| self.get_daily_cached(m)).unwrap_or_default();
        let (today, week, total, reqs) = if let Some(s) = self.summaries.get(self.selected_index) {
            let today_date = chrono::Local::now().format("%Y-%m-%d").to_string();
            let today_tokens = daily
                .iter()
                .find(|(dt, _, _, _, _)| dt == &today_date)
                .map(|(_, i, o, cr, cc)| i + o + cr + cc)
                .unwrap_or(0);
            let week_tokens = daily.iter().map(|(_, i, o, cr, cc)| i + o + cr + cc).sum::<i64>();
            let total_tokens = Self::token_total(s);
            (today_tokens, week_tokens, total_tokens, s.request_count)
        } else {
            (0, 0, 0, 0)
        };

        let card_data = [
            (lang::current().card_today, &format_tokens(today), theme::current().green),
            (lang::current().card_week, &format_tokens(week), theme::current().cyan),
            (lang::current().card_total, &format_tokens(total), theme::current().purple),
            (lang::current().card_reqs, &format!("{}", reqs), theme::current().yellow),
        ];

        for (i, (label, value, color)) in card_data.iter().enumerate() {
            let lines = vec![
                Line::from(Span::styled(*label, Style::default().fg(theme::current().comment))).centered(),
                Line::from(Span::styled(value.to_string(), Style::default().fg(*color))).centered(),
            ];
            let p = Paragraph::new(lines).block(
                Block::bordered()
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .border_style(Style::default().fg(theme::current().dim)),
            );
            f.render_widget(p, cards[i]);
        }
    }

    fn render_profile_list(&mut self, f: &mut Frame, area: Rect) {
        let max = self.max_tokens();
        let items: Vec<ListItem> = self
            .summaries
            .iter()
            .filter(|s| Self::token_total(s) > 0)
            .enumerate()
            .map(|(i, s)| {
                let total = Self::token_total(s);
                let pct = if max > 0 { (total as f64 / max as f64 * 100.0) as usize } else { 0 };
                let bar_len = if total > 0 { (pct / 4).max(1).min(20) } else { 0 };
                let bar = "\u{2500}".repeat(bar_len);
                let label = chart::title_case(&s.model);
                let is_sel = i == self.selected_index;
                let arrow = if is_sel { "\u{276f} " } else { "  " };
                let tc = if is_sel { theme::current().cyan } else { theme::current().fg };
                let bar_text = if total > 0 { format!("{} {}%", bar, pct) } else { String::new() };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{}{}", arrow, label), Style::default().fg(tc)),
                        Span::styled(format!("  {}", format_tokens(total)), Style::default().fg(theme::current().dim)),
                    ]),
                    Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(bar_text, Style::default().fg(theme::current().purple)),
                    ]),
                    Line::from(""),
                ])
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::bordered()
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .title(format!("{} — \u{3a3} {}", lang::current().models_title, format_tokens(self.total_tokens())))
                    .border_style(Style::default().fg(theme::current().dim)),
            )
            .highlight_style(Style::default());
        f.render_stateful_widget(list, area, &mut self.state);
    }

    fn render_daily_chart(&mut self, f: &mut Frame, area: Rect) {
        let model_name = self.summaries.get(self.selected_index).map(|s| s.model.clone());
        let daily = model_name.as_deref().map(|m| self.get_daily_cached(m)).unwrap_or_default();
        if let Some(s) = self.summaries.get(self.selected_index) {
            chart::render_daily_chart(&daily, &s.model, &mut self.chart_scroll, f, area);
        } else {
            // No data — render empty placeholder
            let p = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(lang::current().no_usage, Style::default().fg(theme::current().comment))).centered(),
                Line::from(""),
                Line::from(Span::styled(lang::current().no_usage_hint, Style::default().fg(theme::current().dim))).centered(),
            ]).block(Block::bordered().border_set(ratatui::symbols::border::ROUNDED).title(lang::current().usage_tab_title).border_style(Style::default().fg(theme::current().dim)));
            f.render_widget(p, area);
        }
    }

    fn render_scan_progress(&self, f: &mut Frame, area: Rect, files_done: usize, files_total: usize, records: usize) {
        scan::render_scan_progress(f, area, files_done, files_total, records);
    }
}
