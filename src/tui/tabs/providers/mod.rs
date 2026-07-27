pub mod form;

use super::super::theme;
use super::super::widgets::shared::{
    render_confirm_popup as shared_confirm, render_message_popup as shared_msg,
};
use super::TabContent;
use crate::core::config::ConfigManager;
use crate::core::models::{validate_profile, validate_provider, AppType, Profile, Provider};
use crate::tui::lang;
use crossterm::event::KeyCode;
use form::{EditForm, ProviderForm};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use std::cmp::Ordering;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    ProviderList,
    ProfileList,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ProviderAction {
    Delete,
}

pub struct ProvidersTab {
    mgr: Rc<ConfigManager>,
    app: AppType,
    // Provider list
    providers: Vec<Provider>,
    provider_state: ListState,
    selected_provider_idx: usize,
    // Profile list
    profiles: Vec<Profile>,
    profile_state: ListState,
    selected_profile_idx: usize,
    // Active state
    pub active_provider: String,
    pub active_profile: String,
    // Navigation
    panel: Panel,
    // Search
    pub search_query: String,
    pub is_searching: bool,
    // Popups
    pub confirm_action: Option<ProviderAction>,
    confirm_button: usize,
    pub message: Option<String>,
    status_message: Option<String>,
    edit_form: Option<EditForm>,
    provider_form: Option<ProviderForm>,
}

impl ProvidersTab {
    pub fn new(mgr: Rc<ConfigManager>, app: AppType) -> Self {
        crate::core::sync::sync_active_from_settings(&mgr);
        crate::core::sync::sync_codex_active_from_config(&mgr);

        let mut providers = mgr.list_providers_for(app).unwrap_or_default();
        providers.sort_by(
            |a, b| match (a.source.can_delete(), b.source.can_delete()) {
                (false, true) => Ordering::Less,
                (true, false) => Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            },
        );
        let active_provider = mgr
            .get_setting(app.active_provider_key())
            .unwrap_or_default();
        let active_profile = if app == AppType::Claude {
            mgr.get_setting("active_profile").unwrap_or_default()
        } else {
            String::new()
        };

        let selected_provider_idx = providers
            .iter()
            .position(|p| p.id == active_provider)
            .unwrap_or(0);

        let profiles = if let Some(p) = providers.get(selected_provider_idx) {
            p.profiles.clone()
        } else {
            vec![]
        };

        let selected_profile_idx = if profiles.is_empty() {
            0
        } else {
            profiles
                .iter()
                .position(|pr| pr.id == active_profile)
                .unwrap_or(0)
        };

        let mut provider_state = ListState::default();
        provider_state.select(Some(selected_provider_idx));
        let mut profile_state = ListState::default();
        profile_state.select(if profiles.is_empty() {
            None
        } else {
            Some(selected_profile_idx)
        });

        ProvidersTab {
            mgr,
            app,
            providers,
            provider_state,
            selected_provider_idx,
            profiles,
            profile_state,
            selected_profile_idx,
            active_provider,
            active_profile,
            panel: Panel::ProviderList,
            search_query: String::new(),
            is_searching: false,
            confirm_action: None,
            confirm_button: 0,
            message: None,
            status_message: None,
            edit_form: None,
            provider_form: None,
        }
    }

    pub fn switch_app(&mut self, app: AppType) {
        self.app = app;
        self.panel = Panel::ProviderList;
        self.confirm_action = None;
        self.message = None;
        self.status_message = None;
        self.edit_form = None;
        self.provider_form = None;
        self.selected_provider_idx = 0;
        self.selected_profile_idx = 0;
        self.active_provider = self
            .mgr
            .get_setting(app.active_provider_key())
            .unwrap_or_default();
        self.active_profile = if app == AppType::Claude {
            self.mgr.get_setting("active_profile").unwrap_or_default()
        } else {
            String::new()
        };
        self.refresh_providers();
        if let Some(index) = self
            .providers
            .iter()
            .position(|p| p.id == self.active_provider)
        {
            self.selected_provider_idx = index;
            self.provider_state.select(Some(index));
            self.load_profiles();
        }
    }

    pub fn active_context(&self) -> String {
        if self.active_provider.is_empty() {
            String::new()
        } else if self.app == AppType::Claude && !self.active_profile.is_empty() {
            format!("{}/{}", self.active_provider, self.active_profile)
        } else {
            self.active_provider.clone()
        }
    }

    pub fn status_text(&self) -> String {
        if let Some(message) = &self.status_message {
            return message.clone();
        }
        let provider = self
            .selected_provider()
            .map(|p| p.name.as_str())
            .unwrap_or("-");
        if self.app == AppType::Claude {
            let profile = self
                .selected_profile()
                .map(|p| p.name.as_str())
                .unwrap_or("-");
            format!("Claude · {} / {}", provider, profile)
        } else {
            format!("Codex · {}", provider)
        }
    }

    // ── Provider CRUD ──

    fn do_add_provider(&mut self) {
        self.provider_form = Some(ProviderForm {
            fields: [String::new(), String::new(), String::new(), String::new()],
            cursors: [0, 0, 0, 0],
            focused: 0,
            is_edit: false,
        });
    }

    fn do_edit_provider(&mut self) {
        let Some(prov) = self.selected_provider() else {
            return;
        };
        if !prov.source.can_delete() {
            self.message = Some(lang::current().msg_cannot_edit_sys_provider.into());
            return;
        }
        self.provider_form = Some(ProviderForm {
            fields: [
                prov.name.clone(),
                prov.id.clone(),
                prov.api_url.clone(),
                prov.api_key.clone(),
            ],
            cursors: [
                prov.name.len(),
                prov.id.len(),
                prov.api_url.len(),
                prov.api_key.len(),
            ],
            focused: 0,
            is_edit: true,
        });
    }

    fn commit_provider(&mut self) {
        let Some(form) = self.provider_form.as_ref() else {
            return;
        };
        let pr = Provider {
            id: form.fields[1].clone(),
            name: form.fields[0].clone(),
            api_url: form.fields[2].clone(),
            api_key: form.fields[3].clone(),
            profiles: vec![],
            source: crate::core::models::Source::User,
        };
        if let Err(error) = validate_provider(&pr) {
            self.message = Some(format!("Error: {}", error));
            return;
        }
        if !form.is_edit && self.providers.iter().any(|provider| provider.id == pr.id) {
            self.message = Some(format!("Provider '{}' already exists", pr.id));
            return;
        }
        if let Err(e) = self.mgr.db().insert_provider(&pr, self.app.as_str()) {
            self.message = Some(format!("Failed to save provider: {}", e));
            return;
        }
        self.provider_form = None;
        self.refresh_providers();
        self.status_message = Some(format!("Provider '{}' saved", pr.name));
    }

    fn do_delete_provider(&mut self) {
        let Some(prov) = self.selected_provider() else {
            return;
        };
        if !prov.source.can_delete() {
            self.message = Some(lang::current().msg_cannot_delete_sys_provider.into());
            return;
        }
        let provider_id = prov.id.clone();
        if self.app == AppType::Codex {
            if let Err(e) =
                crate::core::switcher::remove_codex_provider(&self.mgr, &provider_id, None)
            {
                self.message = Some(format!("Failed to update Codex config.toml: {}", e));
                return;
            }
        }
        if let Err(e) = self
            .mgr
            .db()
            .delete_provider(&provider_id, self.app.as_str())
        {
            self.message = Some(format!("Failed to delete: {}", e));
            return;
        }
        if self.active_provider == provider_id {
            self.active_provider.clear();
            self.active_profile.clear();
            let _ = self.mgr.set_setting(self.app.active_provider_key(), "");
            if self.app == AppType::Claude {
                let _ = self.mgr.set_setting("active_profile", "");
            }
        }
        self.refresh_providers();
        self.status_message = Some(format!("Provider '{}' deleted", provider_id));
    }

    /// Full refresh: re-fetch providers from DB (expensive, call on mutations or Enter)
    fn refresh_providers(&mut self) {
        let mut providers = self.mgr.list_providers_for(self.app).unwrap_or_default();
        providers.sort_by(
            |a, b| match (a.source.can_delete(), b.source.can_delete()) {
                (false, true) => Ordering::Less,
                (true, false) => Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            },
        );
        self.providers = providers;
        if self.selected_provider_idx >= self.providers.len() {
            self.selected_provider_idx = 0;
        }
        self.load_profiles();
    }

    /// Lightweight: load profiles for selected provider from cached data (no DB call)
    fn load_profiles(&mut self) {
        self.profiles = if self.app == AppType::Claude {
            if let Some(p) = self.providers.get(self.selected_provider_idx) {
                p.profiles.clone()
            } else {
                vec![]
            }
        } else {
            vec![]
        };
        if self.selected_profile_idx >= self.profiles.len() {
            self.selected_profile_idx = 0;
        }
        self.profile_state.select(if self.profiles.is_empty() {
            None
        } else {
            Some(self.selected_profile_idx)
        });
    }

    fn selected_profile(&self) -> Option<&Profile> {
        self.profiles.get(self.selected_profile_idx)
    }

    fn selected_provider(&self) -> Option<&Provider> {
        self.providers.get(self.selected_provider_idx)
    }

    fn do_add_profile(&mut self) {
        if self.app == AppType::Codex {
            return;
        }
        let prov_id = self
            .selected_provider()
            .map(|p| p.id.clone())
            .unwrap_or_default();
        if prov_id.is_empty() {
            return;
        }
        self.edit_form = Some(EditForm {
            fields: std::array::from_fn(|_| String::new()),
            cursors: [0; 6],
            focused: 0,
            is_edit: false,
            prov_id,
        });
    }

    fn do_edit(&mut self) {
        let Some(prof) = self.selected_profile() else {
            return;
        };
        let fields = [
            prof.id.clone(),
            prof.name.clone(),
            prof.opus.clone(),
            prof.sonnet.clone(),
            prof.haiku.clone(),
            prof.subagent.clone(),
        ];
        let cursors = std::array::from_fn(|index| fields[index].len());
        let prov_id = self
            .selected_provider()
            .map(|p| p.id.clone())
            .unwrap_or_default();
        self.edit_form = Some(EditForm {
            fields,
            cursors,
            focused: 0,
            is_edit: true,
            prov_id,
        });
    }

    fn commit_edit(&mut self) {
        let Some(form) = self.edit_form.as_ref() else {
            return;
        };
        let prof_id = if form.fields[0].is_empty() {
            form.fields[1].to_lowercase().replace(' ', "-")
        } else {
            form.fields[0].clone()
        };
        let pr = Profile {
            id: prof_id,
            name: form.fields[1].clone(),
            opus: form.fields[2].clone(),
            sonnet: form.fields[3].clone(),
            haiku: form.fields[4].clone(),
            subagent: form.fields[5].clone(),
            default: false,
            source: crate::core::models::Source::User,
        };
        if let Err(error) = validate_profile(&pr) {
            self.message = Some(format!("Error: {}", error));
            return;
        }
        if !form.is_edit
            && self
                .providers
                .iter()
                .find(|provider| provider.id == form.prov_id)
                .is_some_and(|provider| provider.profiles.iter().any(|profile| profile.id == pr.id))
        {
            self.message = Some(format!(
                "Profile '{}/{}' already exists",
                form.prov_id, pr.id
            ));
            return;
        }
        if let Err(e) = self.mgr.db().insert_profile(&form.prov_id, &pr) {
            self.message = Some(format!("Failed to save: {}", e));
            tracing::error!("Failed to insert user profile: {}", e);
            return;
        }
        self.edit_form = None;
        self.refresh_providers();
        self.status_message = Some(format!("Profile '{}' saved", pr.name));
    }

    fn do_switch(&mut self) {
        let prov_id = match self.selected_provider() {
            Some(p) => p.id.clone(),
            None => return,
        };
        if self.app == AppType::Codex {
            match crate::core::switcher::switch_codex_provider(&self.mgr, &prov_id, None, None) {
                Ok(_) => {
                    self.active_provider = prov_id.clone();
                    self.status_message = Some(format!("Codex switched to '{}'", prov_id));
                }
                Err(e) => self.message = Some(format!("Error: {}", e)),
            }
            return;
        }
        let prof_id = match self.selected_profile() {
            Some(profile) => profile.id.clone(),
            None => return,
        };
        let mode = if self
            .mgr
            .get_setting("proxy_mode")
            .map(|v| v == "true")
            .unwrap_or(false)
        {
            crate::core::models::SwitchMode::Proxy
        } else {
            crate::core::models::SwitchMode::Local
        };
        if let Err(e) =
            crate::core::switcher::switch_profile(&self.mgr, &prov_id, &prof_id, mode, None)
        {
            self.message = Some(format!("Error: {}", e));
            return;
        }
        self.active_provider = prov_id;
        self.active_profile = prof_id;
        self.status_message = Some(format!(
            "Claude switched to '{}/{}'",
            self.active_provider, self.active_profile
        ));
        if let Err(e) = self
            .mgr
            .set_setting("active_provider", &self.active_provider)
        {
            tracing::error!("Failed to save active_provider: {}", e);
        }
        if let Err(e) = self.mgr.set_setting("active_profile", &self.active_profile) {
            tracing::error!("Failed to save active_profile: {}", e);
        }
    }

    fn do_delete(&mut self) {
        let (provider_id, prof_id) = {
            let Some(prof) = self.selected_profile() else {
                return;
            };
            if !prof.source.can_delete() {
                return;
            }
            let provider_id = self
                .selected_provider()
                .map(|provider| provider.id.clone())
                .unwrap_or_default();
            (provider_id, prof.id.clone())
        };
        if let Err(e) = self.mgr.db().delete_profile(&provider_id, &prof_id) {
            self.message = Some(format!("Failed to delete: {}", e));
            tracing::error!("Failed to delete user profile: {}", e);
            return;
        }
        if self.active_provider == provider_id && self.active_profile == prof_id {
            self.active_profile.clear();
            let _ = self.mgr.set_setting("active_profile", "");
        }
        self.refresh_providers();
        self.status_message = Some(format!("Profile '{}' deleted", prof_id));
    }

    fn render_edit_form(&self, f: &mut Frame, area: Rect) {
        if let Some(ref form) = self.edit_form {
            form::render_edit_form(form, f, area);
        }
    }

    fn render_provider_form(&self, f: &mut Frame, area: Rect) {
        if let Some(ref form) = self.provider_form {
            form::render_provider_form(form, f, area);
        }
    }

    fn render_confirm_popup(&self, f: &mut Frame, area: Rect) {
        let (title, msg, c) = match self.confirm_action {
            Some(ProviderAction::Delete) => {
                if self.panel == Panel::ProviderList {
                    (
                        lang::current().confirm_delete_provider,
                        lang::current().confirm_delete_provider_msg,
                        theme::current().red,
                    )
                } else {
                    (
                        lang::current().confirm_delete_profile,
                        lang::current().confirm_delete_profile_msg,
                        theme::current().red,
                    )
                }
            }
            _ => return,
        };
        shared_confirm(
            f,
            area,
            title,
            msg,
            (
                lang::current().confirm_confirm,
                lang::current().confirm_cancel,
            ),
            (c, self.confirm_button),
        );
    }

    fn render_message_popup(&self, f: &mut Frame, area: Rect) {
        shared_msg(f, area, self.message.as_deref().unwrap_or(""));
    }

    fn render_selection_detail(&self, f: &mut Frame, area: Rect) {
        let mut lines = Vec::new();
        if let Some(provider) = self.selected_provider() {
            lines.push(detail_line(
                "Provider",
                &provider.name,
                theme::current().cyan,
            ));
            lines.push(detail_line("ID", &provider.id, theme::current().fg));
            lines.push(detail_line(
                "Base URL",
                &provider.api_url,
                theme::current().yellow,
            ));
            let key = if provider.api_key.starts_with("env:") {
                provider.api_key.clone()
            } else if provider.api_key.is_empty() {
                "-".into()
            } else {
                "••••••••".into()
            };
            lines.push(detail_line("API key", &key, theme::current().comment));
            if self.app == AppType::Codex {
                lines.push(Line::from(""));
                lines.push(detail_line(
                    "Wire API",
                    "responses",
                    theme::current().purple,
                ));
            } else if let Some(profile) = self.selected_profile() {
                lines.push(Line::from(""));
                lines.push(detail_line("Profile", &profile.name, theme::current().cyan));
                lines.push(detail_line("Opus", &profile.opus, theme::current().fg));
                lines.push(detail_line("Sonnet", &profile.sonnet, theme::current().fg));
                lines.push(detail_line("Haiku", &profile.haiku, theme::current().fg));
                lines.push(detail_line(
                    "Subagent",
                    &profile.subagent,
                    theme::current().fg,
                ));
            }
        } else {
            lines.push(
                Line::from(Span::styled(
                    "No provider selected",
                    Style::default().fg(theme::current().comment),
                ))
                .centered(),
            );
        }
        let detail = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .border_set(ratatui::symbols::border::ROUNDED)
                .title(lang::pick("Configuration Detail", "配置详情"))
                .border_style(Style::default().fg(theme::current().dim)),
        );
        f.render_widget(detail, area);
    }
}

fn detail_line(label: &str, value: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<10}", format!("{}:", label)),
            Style::default().fg(theme::current().purple),
        ),
        Span::styled(value.to_string(), Style::default().fg(color)),
    ])
}

impl TabContent for ProvidersTab {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        let popup_area = f.area();
        let (provider_area, profile_area, detail_area) = if self.app == AppType::Codex {
            if area.width >= 86 {
                let [providers, detail] = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
                    .areas(area);
                (providers, Rect::default(), detail)
            } else {
                let [providers, detail] = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(55), Constraint::Min(8)])
                    .areas(area);
                (providers, Rect::default(), detail)
            }
        } else if area.width >= 96 {
            let [providers, right] = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .areas(area);
            let [profiles, detail] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(52), Constraint::Min(9)])
                .areas(right);
            (providers, profiles, detail)
        } else {
            let [providers, profiles, detail] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(34),
                    Constraint::Percentage(33),
                    Constraint::Min(9),
                ])
                .areas(area);
            (providers, profiles, detail)
        };

        // ── Left: Provider list ──
        let provider_items: Vec<ListItem> = self
            .providers
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let is_sel = self.selected_provider_idx == i;
                let arrow = if is_sel { "› " } else { "  " };
                let tc = if is_sel {
                    theme::current().cyan
                } else {
                    theme::current().fg
                };
                let active = self.active_provider == p.id;
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{}{}", arrow, p.name), Style::default().fg(tc)),
                        if active {
                            Span::styled(" ●", Style::default().fg(theme::current().green))
                        } else {
                            Span::raw("")
                        },
                    ]),
                    Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(&p.id, Style::default().fg(theme::current().comment)),
                        Span::styled(" \u{b7} ", Style::default().fg(theme::current().dim)),
                        Span::styled(
                            source_label(p.source),
                            Style::default().fg(theme::current().comment),
                        ),
                        Span::styled(" \u{b7} ", Style::default().fg(theme::current().dim)),
                        Span::styled(
                            if self.app == AppType::Codex {
                                p.api_url.clone()
                            } else {
                                format!("{} {}", p.profiles.len(), lang::current().profiles_count)
                            },
                            Style::default().fg(theme::current().comment),
                        ),
                    ]),
                    Line::from(""),
                ])
            })
            .collect();

        let prov_list = List::new(provider_items)
            .block(
                Block::bordered()
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .title(format!(
                        "{} ({})",
                        lang::current().providers_title,
                        self.providers.len()
                    ))
                    .border_style(if self.panel == Panel::ProviderList {
                        Style::default().fg(theme::current().cyan)
                    } else {
                        Style::default().fg(theme::current().dim)
                    }),
            )
            .highlight_style(Style::default());
        f.render_stateful_widget(prov_list, provider_area, &mut self.provider_state);

        if self.app == AppType::Codex {
            self.render_selection_detail(f, detail_area);
            if self.provider_form.is_some() {
                self.render_provider_form(f, popup_area);
            }
            if self.confirm_action.is_some() {
                self.render_confirm_popup(f, popup_area);
            }
            if self.message.is_some() {
                self.render_message_popup(f, popup_area);
            }
            return;
        }

        // ── Right: Profile list ──
        let profile_items: Vec<ListItem> = self
            .profiles
            .iter()
            .enumerate()
            .map(|(i, pr)| {
                let is_sel = self.selected_profile_idx == i;
                let arrow = if is_sel { "› " } else { "  " };
                let tc = if is_sel {
                    theme::current().cyan
                } else {
                    theme::current().fg
                };
                let active = self.selected_provider().map(|p| p.id.as_str())
                    == Some(self.active_provider.as_str())
                    && self.active_profile == pr.id;
                let opus = compact_model_name(&pr.opus);
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{}{}", arrow, pr.name), Style::default().fg(tc)),
                        if active {
                            Span::styled(" ●", Style::default().fg(theme::current().green))
                        } else {
                            Span::raw("")
                        },
                    ]),
                    Line::from(vec![
                        Span::styled("     ", Style::default()),
                        Span::styled(&pr.id, Style::default().fg(theme::current().comment)),
                        Span::styled(" \u{b7} ", Style::default().fg(theme::current().dim)),
                        Span::styled(opus, Style::default().fg(theme::current().comment)),
                        Span::styled(" \u{b7} ", Style::default().fg(theme::current().dim)),
                        Span::styled(
                            source_label(pr.source),
                            Style::default().fg(theme::current().comment),
                        ),
                    ]),
                    Line::from(""),
                ])
            })
            .collect();

        if self.profiles.is_empty() {
            let p = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    lang::current().no_profiles,
                    Style::default().fg(theme::current().comment),
                ))
                .centered(),
            ])
            .block(
                Block::bordered()
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .title(format!("{} (0)", lang::current().profiles_title))
                    .border_style(if self.panel == Panel::ProfileList {
                        Style::default().fg(theme::current().cyan)
                    } else {
                        Style::default().fg(theme::current().dim)
                    }),
            );
            f.render_widget(p, profile_area);
        } else {
            let prof_list = List::new(profile_items)
                .block(
                    Block::bordered()
                        .border_set(ratatui::symbols::border::ROUNDED)
                        .title(format!(
                            "{} ({})",
                            lang::current().profiles_title,
                            self.profiles.len()
                        ))
                        .border_style(if self.panel == Panel::ProfileList {
                            Style::default().fg(theme::current().cyan)
                        } else {
                            Style::default().fg(theme::current().dim)
                        }),
                )
                .highlight_style(Style::default());
            f.render_stateful_widget(prof_list, profile_area, &mut self.profile_state);
        }

        self.render_selection_detail(f, detail_area);

        // Popups
        if self.edit_form.is_some() {
            self.render_edit_form(f, popup_area);
        }
        if self.provider_form.is_some() {
            self.render_provider_form(f, popup_area);
        }
        if self.confirm_action.is_some() {
            self.render_confirm_popup(f, popup_area);
        }
        if self.message.is_some() {
            self.render_message_popup(f, popup_area);
        }
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        if self.message.is_some() {
            if matches!(code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')) {
                self.message = None;
            }
            return true;
        }
        // Provider form mode
        if let Some(ref mut f) = self.provider_form {
            match code {
                KeyCode::Esc => {
                    self.provider_form = None;
                }
                KeyCode::Enter => {
                    self.commit_provider();
                }
                _ => {
                    f.handle_key(code);
                }
            }
            return true;
        }
        // Edit form mode
        if let Some(ref mut f) = self.edit_form {
            match code {
                KeyCode::Esc => {
                    self.edit_form = None;
                }
                KeyCode::Enter => {
                    self.commit_edit();
                }
                _ => {
                    f.handle_key(code);
                }
            }
            return true;
        }
        if self.confirm_action.is_some() {
            match code {
                KeyCode::Tab | KeyCode::Right => {
                    self.confirm_button = (self.confirm_button + 1) % 2
                }
                KeyCode::BackTab | KeyCode::Left => {
                    self.confirm_button = if self.confirm_button == 0 { 1 } else { 0 }
                }
                KeyCode::Enter => {
                    if self.confirm_button == 0
                        && self.confirm_action == Some(ProviderAction::Delete)
                    {
                        if self.panel == Panel::ProviderList {
                            self.do_delete_provider();
                        } else {
                            self.do_delete();
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

        match self.panel {
            Panel::ProviderList => self.handle_provider_keys(code),
            Panel::ProfileList => self.handle_profile_keys(code),
        }
    }

    fn shortcut_groups(&self) -> Vec<Vec<(String, Color)>> {
        if self.app == AppType::Codex {
            return vec![
                vec![
                    (" J/K ".into(), theme::current().comment),
                    (lang::current().sc_nav.into(), theme::current().comment),
                ],
                vec![
                    (" ⏎  ".into(), theme::current().comment),
                    (lang::current().sc_switch.into(), theme::current().comment),
                ],
                vec![
                    (" A ".into(), theme::current().comment),
                    (lang::current().sc_add.into(), theme::current().comment),
                ],
                vec![
                    (" E ".into(), theme::current().comment),
                    (lang::current().sc_edit.into(), theme::current().comment),
                ],
                vec![
                    (" D ".into(), theme::current().comment),
                    (lang::current().sc_delete.into(), theme::current().comment),
                ],
                vec![
                    (" Q ".into(), theme::current().comment),
                    (lang::current().sc_quit.into(), theme::current().comment),
                ],
            ];
        }
        match self.panel {
            Panel::ProviderList => vec![
                vec![
                    (" J/K ".into(), theme::current().comment),
                    (lang::current().sc_nav.into(), theme::current().comment),
                ],
                vec![
                    (" L/→ ".into(), theme::current().comment),
                    (lang::current().sc_profiles.into(), theme::current().comment),
                ],
                vec![
                    (" A ".into(), theme::current().comment),
                    (lang::current().sc_add.into(), theme::current().comment),
                ],
                vec![
                    (" E ".into(), theme::current().comment),
                    (lang::current().sc_edit.into(), theme::current().comment),
                ],
                vec![
                    (" D ".into(), theme::current().comment),
                    (lang::current().sc_delete.into(), theme::current().comment),
                ],
                vec![
                    (" Q ".into(), theme::current().comment),
                    (lang::current().sc_quit.into(), theme::current().comment),
                ],
            ],
            Panel::ProfileList => vec![
                vec![
                    (" J/K ".into(), theme::current().comment),
                    (lang::current().sc_nav.into(), theme::current().comment),
                ],
                vec![
                    (" H/← ".into(), theme::current().comment),
                    (lang::current().sc_back.into(), theme::current().comment),
                ],
                vec![
                    (" ⏎  ".into(), theme::current().comment),
                    (lang::current().sc_switch.into(), theme::current().comment),
                ],
                vec![
                    (" A ".into(), theme::current().comment),
                    (lang::current().sc_add.into(), theme::current().comment),
                ],
                vec![
                    (" D ".into(), theme::current().comment),
                    (lang::current().sc_delete.into(), theme::current().comment),
                ],
                vec![
                    (" E ".into(), theme::current().comment),
                    (lang::current().sc_edit.into(), theme::current().comment),
                ],
                vec![
                    (" Q ".into(), theme::current().comment),
                    (lang::current().sc_quit.into(), theme::current().comment),
                ],
            ],
        }
    }
}

// ── Key handlers ──

impl ProvidersTab {
    fn handle_provider_keys(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Tab | KeyCode::BackTab => return false,
            KeyCode::Char('j') | KeyCode::Down => {
                let l = self.providers.len();
                if l > 0 {
                    self.selected_provider_idx = if self.selected_provider_idx + 1 < l {
                        self.selected_provider_idx + 1
                    } else {
                        0
                    };
                    self.provider_state.select(Some(self.selected_provider_idx));
                    self.load_profiles();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let l = self.providers.len();
                if l > 0 {
                    self.selected_provider_idx = if self.selected_provider_idx > 0 {
                        self.selected_provider_idx - 1
                    } else {
                        l - 1
                    };
                    self.provider_state.select(Some(self.selected_provider_idx));
                    self.load_profiles();
                }
            }
            KeyCode::Enter => {
                if self.app == AppType::Codex {
                    if !self.providers.is_empty() {
                        self.do_switch();
                    }
                } else {
                    self.panel = Panel::ProfileList;
                    self.refresh_providers();
                    self.profile_state.select(Some(self.selected_profile_idx));
                }
            }
            KeyCode::Char('l') | KeyCode::Right if self.app == AppType::Claude => {
                self.panel = Panel::ProfileList;
                self.profile_state.select(if self.profiles.is_empty() {
                    None
                } else {
                    Some(self.selected_profile_idx)
                });
            }
            KeyCode::Char('a') | KeyCode::Char('A') => self.do_add_provider(),
            KeyCode::Char('e') | KeyCode::Char('E') => self.do_edit_provider(),
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if let Some(prov) = self.selected_provider() {
                    if !prov.source.can_delete() {
                        self.message = Some(lang::current().msg_cannot_delete_sys_provider.into());
                    } else {
                        self.confirm_action = Some(ProviderAction::Delete);
                        self.confirm_button = 0;
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn handle_profile_keys(&mut self, code: KeyCode) -> bool {
        if self.is_searching {
            match code {
                KeyCode::Esc => {
                    self.is_searching = false;
                    self.search_query.clear();
                }
                KeyCode::Enter => self.is_searching = false,
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
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.panel = Panel::ProviderList;
                self.provider_state.select(Some(self.selected_provider_idx));
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let l = self.profiles.len();
                if l > 0 {
                    self.selected_profile_idx = if self.selected_profile_idx + 1 < l {
                        self.selected_profile_idx + 1
                    } else {
                        0
                    };
                    self.profile_state.select(Some(self.selected_profile_idx));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let l = self.profiles.len();
                if l > 0 {
                    self.selected_profile_idx = if self.selected_profile_idx > 0 {
                        self.selected_profile_idx - 1
                    } else {
                        l - 1
                    };
                    self.profile_state.select(Some(self.selected_profile_idx));
                }
            }
            KeyCode::Enter => {
                if !self.profiles.is_empty() {
                    self.do_switch();
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if self.profiles.is_empty() {
                    return false;
                }
                if let Some(pr) = self.selected_profile() {
                    if !pr.source.can_delete() {
                        self.message = Some(lang::current().msg_cannot_delete_sys_profile.into());
                        return true;
                    }
                }
                self.confirm_action = Some(ProviderAction::Delete);
                self.confirm_button = 0;
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                if self.profiles.is_empty() {
                    return false;
                }
                if let Some(pr) = self.selected_profile() {
                    if !pr.source.can_delete() {
                        self.message = Some(lang::current().msg_cannot_edit_sys_profile.into());
                        return true;
                    }
                }
                self.do_edit();
            }
            KeyCode::Char('a') | KeyCode::Char('A') => self.do_add_profile(),
            _ => return false,
        }
        true
    }
}

fn source_label(s: crate::core::models::Source) -> &'static str {
    if s.can_delete() {
        lang::current().label_user
    } else {
        lang::current().label_system
    }
}

fn compact_model_name(model: &str) -> String {
    let model = model.trim();
    if model.is_empty() {
        "-".into()
    } else {
        model
            .replace("[1m]", " 1M")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::compact_model_name;

    #[test]
    fn formats_profile_opus_model_for_list() {
        assert_eq!(
            compact_model_name("deepseek-v4-pro[1m]"),
            "deepseek-v4-pro 1M"
        );
        assert_eq!(compact_model_name(" claude-opus-4 "), "claude-opus-4");
        assert_eq!(compact_model_name(""), "-");
    }
}
