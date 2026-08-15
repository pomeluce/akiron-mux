use super::super::lang;
use super::super::theme::{self, THEMES};
use super::super::widgets::shared::display_width;
use super::TabContent;
use crate::core::config::{self, ConfigManager};
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};
use std::rc::Rc;

const MODES: &[&str] = &["local", "proxy"];

pub struct SettingsTab {
    mgr: Rc<ConfigManager>,
    selected: usize,
    theme_idx: usize,
    mode_idx: usize,
    lang_idx: usize,
    session_service_enabled: bool,
}

impl SettingsTab {
    pub fn new(mgr: Rc<ConfigManager>) -> Self {
        // Restore theme from DB
        let saved_theme = mgr.get_setting("theme").unwrap_or_default();
        if !saved_theme.is_empty() {
            theme::set_theme(&saved_theme);
        }
        let current_theme = theme::current_theme();
        let theme_idx = THEMES.iter().position(|&t| t == current_theme).unwrap_or(0);

        // Restore language from DB
        let saved_lang = mgr.get_setting("language").unwrap_or_default();
        let lang_idx = if saved_lang.is_empty() {
            0
        } else {
            lang::set_lang(&saved_lang);
            lang::LANGS.iter().position(|(n, _)| *n == saved_lang).unwrap_or(0)
        };

        let mode_idx = if mgr.get_setting("proxy_mode").map(|v| v == "true").unwrap_or(false) { 1 } else { 0 };
        let session_service_enabled = crate::session_service::control::enabled(&mgr);
        if let Err(error) = crate::session_service::control::reconcile(&mgr) {
            tracing::warn!("Failed to reconcile AkironMux session service: {error:#}");
        }
        SettingsTab {
            mgr,
            selected: 0,
            theme_idx,
            mode_idx,
            lang_idx,
            session_service_enabled,
        }
    }

    fn items(&self) -> Vec<(&str, String)> {
        let l = lang::current();
        vec![
            (l.setting_theme, THEMES[self.theme_idx].to_string()),
            (l.setting_language, lang::current_lang().to_string()),
            (l.setting_mode, MODES[self.mode_idx].to_string()),
            (
                l.setting_session_service,
                lang::pick(
                    if self.session_service_enabled { "Enabled" } else { "Disabled" },
                    if self.session_service_enabled { "开启" } else { "关闭" },
                )
                .to_string(),
            ),
        ]
    }

    pub fn status_text(&self) -> String {
        format!(
            "{} · {} · {} · {} · {}",
            lang::pick("Shared settings", "共享设置"),
            THEMES[self.theme_idx],
            lang::current_lang(),
            MODES[self.mode_idx],
            lang::pick(
                if self.session_service_enabled { "backend on" } else { "backend off" },
                if self.session_service_enabled { "后端开启" } else { "后端关闭" }
            )
        )
    }

    fn cycle_theme(&mut self, forward: bool) {
        self.theme_idx = if forward {
            (self.theme_idx + 1) % THEMES.len()
        } else if self.theme_idx == 0 {
            THEMES.len() - 1
        } else {
            self.theme_idx - 1
        };
        theme::set_theme(THEMES[self.theme_idx]);
        if let Err(e) = self.mgr.set_setting("theme", THEMES[self.theme_idx]) {
            tracing::warn!("Failed to save theme: {}", e);
        }
    }

    fn cycle_mode(&mut self, forward: bool) {
        self.mode_idx = if forward {
            (self.mode_idx + 1) % MODES.len()
        } else if self.mode_idx == 0 {
            MODES.len() - 1
        } else {
            self.mode_idx - 1
        };
        let is_proxy = self.mode_idx == 1;
        if let Err(e) = self.mgr.set_setting("proxy_mode", &is_proxy.to_string()) {
            tracing::warn!("Failed to save mode: {}", e);
        }

        // Immediately apply the mode change to settings.json if a profile is active
        let mode = if is_proxy {
            crate::core::models::SwitchMode::Proxy
        } else {
            crate::core::models::SwitchMode::Local
        };
        if let (Some(prov_id), Some(prof_id)) = (self.mgr.get_setting("active_provider"), self.mgr.get_setting("active_profile")) {
            if let Err(e) = crate::core::switcher::switch_profile(&self.mgr, &prov_id, &prof_id, mode, None) {
                tracing::warn!("Failed to apply mode switch: {}", e);
            }
        }
    }

    fn cycle_lang(&mut self, forward: bool) {
        let n = lang::LANGS.len();
        self.lang_idx = if forward {
            (self.lang_idx + 1) % n
        } else if self.lang_idx == 0 {
            n - 1
        } else {
            self.lang_idx - 1
        };
        let name = lang::LANGS[self.lang_idx].0;
        lang::set_lang(name);
        if let Err(e) = self.mgr.set_setting("language", name) {
            tracing::warn!("Failed to save language: {}", e);
        }
    }

    fn toggle_session_service(&mut self) {
        let next = !self.session_service_enabled;
        match crate::session_service::control::set_enabled(&self.mgr, next) {
            Ok(()) => self.session_service_enabled = next,
            Err(error) => tracing::warn!("Failed to update AkironMux session service: {error:#}"),
        }
    }
}

impl TabContent for SettingsTab {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        let items = self.items();
        let refresh_label = lang::pick("Session Refresh", "会话刷新");
        let database_label = lang::pick("Database", "数据库");
        let max_label_dw = items
            .iter()
            .map(|(label, _)| display_width(label))
            .chain([display_width(refresh_label), display_width(database_label)])
            .max()
            .unwrap_or(0);
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Length(5), Constraint::Length(5), Constraint::Min(5)])
            .split(area);

        let appearance = vec![
            setting_line(0, self.selected, items[0].0, &items[0].1, max_label_dw),
            Line::from(""),
            setting_line(1, self.selected, items[1].0, &items[1].1, max_label_dw),
        ];
        f.render_widget(
            section(format!("{} · {}", lang::pick("Appearance", "外观"), lang::current().settings_title), appearance),
            sections[0],
        );

        let claude = vec![setting_line(2, self.selected, items[2].0, &items[2].1, max_label_dw)];
        f.render_widget(section("Claude".into(), claude), sections[1]);

        let backend = vec![setting_line(3, self.selected, items[3].0, &items[3].1, max_label_dw)];
        f.render_widget(section(lang::pick("Services", "服务").into(), backend), sections[2]);

        let database = shorten_home(&config::db_path().display().to_string());
        let data = vec![
            readonly_line(refresh_label, lang::pick("Real-time", "实时"), max_label_dw),
            readonly_line(database_label, &database, max_label_dw),
        ];
        f.render_widget(section(lang::pick("Data", "数据").into(), data), sections[3]);
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Tab | KeyCode::BackTab => return false,
            KeyCode::Char('j') | KeyCode::Down => {
                let max = self.items().len().saturating_sub(1);
                self.selected = (self.selected + 1).min(max);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Char('l') | KeyCode::Right => match self.selected {
                0 => self.cycle_theme(true),
                1 => self.cycle_lang(true),
                2 => self.cycle_mode(true),
                3 => self.toggle_session_service(),
                _ => {}
            },
            KeyCode::Char('h') | KeyCode::Left => match self.selected {
                0 => self.cycle_theme(false),
                1 => self.cycle_lang(false),
                2 => self.cycle_mode(false),
                3 => self.toggle_session_service(),
                _ => {}
            },
            _ => return false,
        }
        true
    }

    fn shortcut_groups(&self) -> Vec<Vec<(String, Color)>> {
        let l = lang::current();
        vec![
            vec![(" J/K ".into(), theme::current().comment), (l.sc_nav.into(), theme::current().comment)],
            vec![(" H/L ".into(), theme::current().comment), (l.sc_toggle.into(), theme::current().comment)],
            vec![(" Q ".into(), theme::current().comment), (l.sc_quit.into(), theme::current().comment)],
        ]
    }
}

fn section<'a>(title: String, lines: Vec<Line<'a>>) -> Paragraph<'a> {
    Paragraph::new(lines).block(
        Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .title(title)
            .border_style(Style::default().fg(theme::current().dim)),
    )
}

fn setting_line<'a>(index: usize, selected: usize, label: &'a str, value: &'a str, width: usize) -> Line<'a> {
    let active = index == selected;
    Line::from(vec![
        Span::styled(if active { "› " } else { "  " }, Style::default().fg(theme::current().cyan)),
        Span::styled(
            settings_label(label, width),
            Style::default().fg(if active { theme::current().cyan } else { theme::current().fg }),
        ),
        Span::styled(
            format!("<{}>", value),
            Style::default().fg(if active { theme::current().purple } else { theme::current().dim }),
        ),
    ])
}

fn readonly_line<'a>(label: &'a str, value: &'a str, width: usize) -> Line<'a> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(settings_label(label, width), Style::default().fg(theme::current().fg)),
        Span::styled(value, Style::default().fg(theme::current().comment)),
    ])
}

fn settings_label(label: &str, width: usize) -> String {
    let label_width = display_width(label);
    if label_width >= width {
        format!("{}: ", label)
    } else {
        format!("{}{}: ", label, " ".repeat(width - label_width))
    }
}

fn shorten_home(path: &str) -> String {
    std::env::var("HOME").ok().map(|home| path.replacen(&home, "~", 1)).unwrap_or_else(|| path.to_string())
}
