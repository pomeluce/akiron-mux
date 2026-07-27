pub mod chart;
use crate::tui::lang;

use super::super::theme;
use super::super::widgets::shared::{format_tokens, render_search_box as shared_search};
use super::TabContent;
use crate::core::config::ConfigManager;
use crate::core::models::AppType;
use crate::db::usage::{DailyUsage, ScanContext, ScanEvent, UsageSummary};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::rc::Rc;
use std::sync::mpsc;

/// Background scan state, updated by poll_scan_events()
enum ScanState {
    Idle,
    Scanning {
        files_done: usize,
        files_total: usize,
        records: usize,
    },
}

pub struct UsageTab {
    mgr: Rc<ConfigManager>,
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
    cached_daily: Option<(String, Vec<DailyUsage>)>,
    /// Background scan receiver + state
    scan_rx: Option<mpsc::Receiver<ScanEvent>>,
    scan_state: ScanState,
    /// Handle for graceful shutdown of the background scan thread
    scan_handle: Option<std::thread::JoinHandle<()>>,
    /// A file change arrived while a scan was running.
    rescan_pending: bool,
}

impl UsageTab {
    pub fn new(mgr: Rc<ConfigManager>, app: AppType) -> Self {
        let scan_state;
        let scan_rx;
        let scan_handle;
        let app_type = app.as_str().to_string();

        // Check if this is first launch (no usage data yet)
        let is_first_launch = {
            let db = mgr.db();
            let count: i64 = db
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM usage_logs WHERE app_type = ?1",
                    [&app_type],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            count == 0
        };

        // Prepare scan context on main thread (DB access only, fast) then spawn background parser
        {
            let (tx, rx) = mpsc::channel();
            let ctx = match mgr.db().prepare_scan_context() {
                Ok(c) => {
                    tracing::info!("Scan prep: {} files in index", c.file_index.len());
                    c
                }
                Err(e) => {
                    tracing::error!("Failed to prepare scan context: {}", e);
                    ScanContext {
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
        self.summaries = self
            .mgr
            .db()
            .query_usage(&self.app_type, &self.range)
            .unwrap_or_default();
        self.sync_visible_selection();
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

    pub fn status_text(&self) -> String {
        match self.scan_state {
            ScanState::Scanning {
                files_done,
                files_total,
                records,
            } => {
                format!(
                    "{} · {} {}/{} {} · {} {}",
                    self.app_type,
                    lang::pick("scanning", "扫描中"),
                    files_done,
                    files_total,
                    lang::pick("files", "文件"),
                    records,
                    lang::pick("records", "记录")
                )
            }
            ScanState::Idle => format!(
                "{} · {} · {} {}",
                self.app_type,
                lang::pick("live", "实时"),
                self.visible_indices().len(),
                lang::pick("models", "模型")
            ),
        }
    }

    /// Trigger an incremental scan (called by file watcher when changes detected)
    pub fn trigger_incremental_scan(&mut self) {
        if self.scan_handle.is_some() {
            self.rescan_pending = true;
            return;
        }
        let scan_app = self.app_type.clone();
        let ctx = match self.mgr.db().prepare_scan_context() {
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
        self.summaries.iter().map(Self::token_total).sum()
    }
    fn visible_indices(&self) -> Vec<usize> {
        let query = self.search_query.trim().to_lowercase();
        self.summaries
            .iter()
            .enumerate()
            .filter(|(_, summary)| {
                Self::token_total(summary) > 0
                    && (query.is_empty() || summary.model.to_lowercase().contains(&query))
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn sync_visible_selection(&mut self) {
        let visible = self.visible_indices();
        if visible.is_empty() {
            self.selected_index = 0;
            self.state.select(None);
            return;
        }
        if let Some(position) = visible
            .iter()
            .position(|index| *index == self.selected_index)
        {
            self.state.select(Some(position));
        } else {
            self.selected_index = visible[0];
            self.state.select(Some(0));
        }
    }

    fn selected_summary(&self) -> Option<&UsageSummary> {
        self.state.selected()?;
        self.summaries.get(self.selected_index)
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
                Ok(ScanEvent::Batch {
                    app_type,
                    sid,
                    file_path,
                    records,
                }) => {
                    if !records.is_empty() {
                        if let Err(e) = self
                            .mgr
                            .db()
                            .insert_usage_batch(&app_type, &sid, &file_path, &records)
                        {
                            tracing::error!("Failed to insert usage batch: {}", e);
                        }
                    }
                }
                Ok(ScanEvent::Progress {
                    files_done,
                    files_total,
                    records,
                }) => {
                    if matches!(self.scan_state, ScanState::Scanning { .. }) {
                        self.scan_state = ScanState::Scanning {
                            files_done,
                            files_total,
                            records,
                        };
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
            self.summaries = self
                .mgr
                .db()
                .query_usage(&self.app_type, &self.range)
                .unwrap_or_default();
            self.sync_visible_selection();
            if self.rescan_pending || completed_app != self.app_type {
                self.trigger_incremental_scan();
            }
        }
    }
}

impl TabContent for UsageTab {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        if area.width >= 92 {
            let [left, chart] = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(area);
            let [search, cards, ranking] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(4),
                    Constraint::Min(3),
                ])
                .areas(left);
            self.render_search_box(f, search);
            self.render_summary_cards(f, cards);
            self.render_profile_list(f, ranking);
            self.render_daily_chart(f, chart);
        } else {
            let card_height = 4;
            let [search, cards, ranking, chart] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(card_height),
                    Constraint::Percentage(38),
                    Constraint::Min(8),
                ])
                .areas(area);
            self.render_search_box(f, search);
            self.render_summary_cards(f, cards);
            self.render_profile_list(f, ranking);
            self.render_daily_chart(f, chart);
        }
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        if self.is_searching {
            match code {
                KeyCode::Esc => {
                    self.is_searching = false;
                    self.search_query.clear();
                    self.sync_visible_selection();
                }
                KeyCode::Enter => {
                    self.is_searching = false;
                    self.sync_visible_selection();
                }
                KeyCode::Backspace | KeyCode::Delete => {
                    self.search_query.pop();
                    self.sync_visible_selection();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.sync_visible_selection();
                }
                _ => {}
            }
            return true;
        }
        match code {
            KeyCode::Tab | KeyCode::BackTab => return false,
            KeyCode::Char('j') | KeyCode::Down => {
                let visible = self.visible_indices();
                if let Some(position) = visible
                    .iter()
                    .position(|index| *index == self.selected_index)
                {
                    let next = (position + 1).min(visible.len().saturating_sub(1));
                    self.selected_index = visible[next];
                    self.state.select(Some(next));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let visible = self.visible_indices();
                if let Some(position) = visible
                    .iter()
                    .position(|index| *index == self.selected_index)
                {
                    let previous = position.saturating_sub(1);
                    self.selected_index = visible[previous];
                    self.state.select(Some(previous));
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
                self.summaries = self
                    .mgr
                    .db()
                    .query_usage(&self.app_type, &self.range)
                    .unwrap_or_default();
                self.sync_visible_selection();
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
            vec![
                (" J/K ".into(), theme::current().comment),
                (lang::current().sc_nav.into(), theme::current().comment),
            ],
            vec![
                (" / ".into(), theme::current().comment),
                (lang::current().sc_search.into(), theme::current().comment),
            ],
            vec![
                (" T ".into(), theme::current().comment),
                (lang::current().sc_toggle.into(), theme::current().comment),
            ],
            vec![
                (" PgUp/Dn ".into(), theme::current().comment),
                (lang::current().sc_scroll.into(), theme::current().comment),
            ],
            vec![
                (" Q ".into(), theme::current().comment),
                (lang::current().sc_quit.into(), theme::current().comment),
            ],
        ]
    }
}

impl UsageTab {
    fn render_search_box(&self, f: &mut Frame, area: Rect) {
        shared_search(f, area, &self.search_query, self.is_searching);
    }

    fn get_daily_cached(&mut self, model: &str) -> Vec<(String, i64, i64, i64, i64)> {
        let key = format!("{}|{}", self.app_type, model);
        if let Some((ref k, ref data)) = self.cached_daily {
            if k == &key {
                return data.clone();
            }
        }
        let data = self
            .mgr
            .db()
            .query_daily_usage(&self.app_type, model)
            .unwrap_or_default();
        self.cached_daily = Some((key, data.clone()));
        data
    }

    fn render_summary_cards(&mut self, f: &mut Frame, area: Rect) {
        let cards = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 4); 4])
            .split(area);

        let model_name = self.selected_summary().map(|s| s.model.clone());
        let daily = model_name
            .as_deref()
            .map(|m| self.get_daily_cached(m))
            .unwrap_or_default();
        let (today, week, total, reqs) = if let Some(s) = self.selected_summary() {
            let today_date = chrono::Local::now().format("%Y-%m-%d").to_string();
            let today_tokens = daily
                .iter()
                .find(|(dt, _, _, _, _)| dt == &today_date)
                .map(|(_, i, o, cr, cc)| i + o + cr + cc)
                .unwrap_or(0);
            let week_tokens = daily
                .iter()
                .map(|(_, i, o, cr, cc)| i + o + cr + cc)
                .sum::<i64>();
            let total_tokens = Self::token_total(s);
            (today_tokens, week_tokens, total_tokens, s.request_count)
        } else {
            (0, 0, 0, 0)
        };

        let card_data = [
            (
                lang::current().card_today,
                &format_tokens(today),
                theme::current().green,
            ),
            (
                lang::current().card_week,
                &format_tokens(week),
                theme::current().cyan,
            ),
            (
                lang::current().card_total,
                &format_tokens(total),
                theme::current().purple,
            ),
            (
                lang::current().card_reqs,
                &format!("{}", reqs),
                theme::current().yellow,
            ),
        ];

        for (i, (label, value, color)) in card_data.iter().enumerate() {
            let lines = vec![
                Line::from(Span::styled(
                    *label,
                    Style::default().fg(theme::current().comment),
                ))
                .centered(),
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
        self.sync_visible_selection();
        let visible = self.visible_indices();
        let max = visible
            .iter()
            .map(|index| Self::token_total(&self.summaries[*index]))
            .max()
            .unwrap_or(1);
        let items: Vec<ListItem> = visible
            .iter()
            .map(|index| {
                let s = &self.summaries[*index];
                let total = Self::token_total(s);
                let pct = if max > 0 {
                    (total as f64 / max as f64 * 100.0) as usize
                } else {
                    0
                };
                let bar_len = if total > 0 { (pct / 4).clamp(1, 20) } else { 0 };
                let bar = "\u{2500}".repeat(bar_len);
                let label = chart::title_case(&s.model);
                let is_sel = *index == self.selected_index;
                let arrow = if is_sel { "\u{276f} " } else { "  " };
                let tc = if is_sel {
                    theme::current().cyan
                } else {
                    theme::current().fg
                };
                let bar_text = if total > 0 {
                    format!("{} {}%", bar, pct)
                } else {
                    String::new()
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{}{}", arrow, label), Style::default().fg(tc)),
                        Span::styled(
                            format!("  {}", format_tokens(total)),
                            Style::default().fg(theme::current().dim),
                        ),
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
                    .title(format!(
                        "{} — \u{3a3} {}",
                        lang::current().models_title,
                        format_tokens(self.total_tokens())
                    ))
                    .border_style(Style::default().fg(theme::current().dim)),
            )
            .highlight_style(Style::default());
        f.render_stateful_widget(list, area, &mut self.state);
    }

    fn render_daily_chart(&mut self, f: &mut Frame, area: Rect) {
        let model_name = self.selected_summary().map(|s| s.model.clone());
        let daily = model_name
            .as_deref()
            .map(|m| self.get_daily_cached(m))
            .unwrap_or_default();
        if let Some(model) = model_name {
            chart::render_daily_chart(&daily, &model, &mut self.chart_scroll, f, area);
        } else {
            // No data — render empty placeholder
            let p = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    lang::current().no_usage,
                    Style::default().fg(theme::current().comment),
                ))
                .centered(),
                Line::from(""),
                Line::from(Span::styled(
                    lang::current().no_usage_hint,
                    Style::default().fg(theme::current().dim),
                ))
                .centered(),
            ])
            .block(
                Block::bordered()
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .title(lang::current().usage_tab_title)
                    .border_style(Style::default().fg(theme::current().dim)),
            );
            f.render_widget(p, area);
        }
    }
}
