use super::super::lang;
use super::super::theme::{self, THEMES};
use super::super::widgets::shared::display_width;
use super::TabContent;
use crate::core::agent_configuration::AgentConfiguration;
use crate::core::config::{self, ConfigManager};
use crossterm::event::KeyCode;
use qrcode::{render::unicode, QrCode};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};
use std::{rc::Rc, sync::mpsc};

use crate::{
    db::remote::BackendDevice,
    session_service::remote::{PairingOffer, PendingPairingInfo, RemoteBackendConfig},
};

const MODES: &[&str] = &["local", "proxy"];

enum RemoteActionResult {
    PairCreated(Result<PairingOffer, String>),
    PairingsLoaded(Result<Vec<PendingPairingInfo>, String>),
    PairConfirmed(Result<(), String>),
    PairCancelled(Result<(), String>),
    DeviceRevoked(Result<(), String>),
    Failed(String),
}

pub struct SettingsTab {
    mgr: Rc<ConfigManager>,
    selected: usize,
    theme_idx: usize,
    mode_idx: usize,
    lang_idx: usize,
    session_service_enabled: bool,
    remote_backend_enabled: bool,
    remote_listener_status: String,
    pairing_offer: Option<PairingOffer>,
    pending_pairing: Option<PendingPairingInfo>,
    devices: Vec<BackendDevice>,
    selected_device: usize,
    pending_revoke: bool,
    remote_notice: String,
    remote_action_rx: Option<mpsc::Receiver<RemoteActionResult>>,
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
        let remote_backend_enabled = mgr.get_setting("remote.enabled").as_deref() == Some("true");
        if let Err(error) = crate::session_service::control::reconcile(&mgr) {
            tracing::warn!("Failed to reconcile AkironMux session service: {error:#}");
        }
        let mut tab = SettingsTab {
            mgr,
            selected: 0,
            theme_idx,
            mode_idx,
            lang_idx,
            session_service_enabled,
            remote_backend_enabled,
            remote_listener_status: String::new(),
            pairing_offer: None,
            pending_pairing: None,
            devices: Vec::new(),
            selected_device: 0,
            pending_revoke: false,
            remote_notice: String::new(),
            remote_action_rx: None,
        };
        tab.refresh_remote_state();
        tab
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
            (
                lang::pick("Remote backend", "远程后端"),
                lang::pick(
                    if self.remote_backend_enabled { "Enabled" } else { "Disabled" },
                    if self.remote_backend_enabled { "开启" } else { "关闭" },
                )
                .to_string(),
            ),
            (lang::pick("Remote listener", "远程监听"), self.remote_listener_status.clone()),
            (lang::pick("Pair device", "设备配对"), lang::pick("Enter to create", "回车创建").to_string()),
            (lang::pick("Remote device", "远程设备"), self.selected_device_label()),
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
            if let Err(e) = AgentConfiguration::new(&self.mgr).apply_claude_profile(&prov_id, &prof_id, mode) {
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
            Ok(()) => {
                self.session_service_enabled = next;
                self.refresh_remote_state();
            }
            Err(error) => tracing::warn!("Failed to update AkironMux session service: {error:#}"),
        }
    }

    fn toggle_remote_backend(&mut self) {
        let next = !self.remote_backend_enabled;
        if next {
            let configured = self.mgr.get_setting("remote.public_url").is_some_and(|value| !value.trim().is_empty());
            let has_device = self.mgr.db().has_active_backend_device().unwrap_or(false);
            if !configured {
                self.remote_notice = lang::pick("Configure the Remote public URL before enabling the listener", "请先配置远程公网地址，再开启监听").to_string();
                return;
            }
            if !has_device {
                self.remote_notice = lang::pick("Create or pair a Remote device before enabling the listener", "请先创建或配对远程设备，再开启监听").to_string();
                return;
            }
            if !self.session_service_enabled {
                if let Err(error) = crate::session_service::control::set_enabled(&self.mgr, true) {
                    self.remote_notice = format!("{}: {error}", lang::pick("Unable to start the session backend", "无法启动会话后端"));
                    return;
                }
                self.session_service_enabled = true;
            }
        }
        match self.mgr.set_setting("remote.enabled", &next.to_string()) {
            Ok(()) => {
                self.remote_backend_enabled = next;
                self.remote_notice = lang::pick(
                    if next { "Remote listener is starting" } else { "Remote listener disabled" },
                    if next { "远程监听正在启动" } else { "远程监听已关闭" },
                )
                .to_string();
                self.refresh_remote_state();
            }
            Err(error) => {
                self.remote_notice = format!("{}: {error}", lang::pick("Unable to update the Remote listener", "无法更新远程监听"));
            }
        }
    }

    fn refresh_remote_state(&mut self) {
        self.devices = self
            .mgr
            .db()
            .list_backend_devices()
            .unwrap_or_default()
            .into_iter()
            .filter(|device| device.revoked_at_ms.is_none())
            .collect();
        self.selected_device = self.selected_device.min(self.devices.len().saturating_sub(1));
        self.remote_listener_status = match RemoteBackendConfig::load(self.mgr.db()) {
            Ok(config) if !config.enabled => lang::pick("Disabled", "已关闭").to_string(),
            Ok(config) => match config.listener(self.mgr.db()) {
                Ok(Some(listener)) => {
                    let reachable = std::net::TcpStream::connect_timeout(&listener.bind, std::time::Duration::from_millis(75)).is_ok();
                    lang::pick(if reachable { "Listening" } else { "Starting" }, if reachable { "监听中" } else { "启动中" }).to_string()
                }
                Ok(None) => lang::pick("Disabled", "已关闭").to_string(),
                Err(_) => lang::pick("Invalid config", "配置无效").to_string(),
            },
            Err(_) => lang::pick("Invalid config", "配置无效").to_string(),
        };
        self.remote_backend_enabled = self.mgr.get_setting("remote.enabled").as_deref() == Some("true");
    }

    fn selected_device_label(&self) -> String {
        self.devices.get(self.selected_device).map_or_else(
            || lang::pick("No active devices", "没有活动设备").to_string(),
            |device| {
                if self.pending_revoke {
                    format!("{} · {}", device.name, lang::pick("press X again", "再次按 X"))
                } else {
                    device.name.clone()
                }
            },
        )
    }

    fn create_pairing(&mut self) {
        if self.remote_action_rx.is_some() {
            return;
        }
        if !crate::session_service::control::is_running() {
            self.remote_notice = lang::pick("Start the session backend before pairing", "请先启动会话后端再配对").to_string();
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.remote_action_rx = Some(rx);
        self.remote_notice = lang::pick("Creating pairing…", "正在创建配对…").to_string();
        std::thread::spawn(move || {
            let client = crate::session_service::admin::LocalAdminClient::from_env();
            let result = tokio::runtime::Runtime::new()
                .map_err(|error| error.to_string())
                .and_then(|runtime| runtime.block_on(client.create_pairing()).map_err(|error| error.to_string()));
            let _ = tx.send(RemoteActionResult::PairCreated(result));
        });
    }

    fn refresh_pairing(&mut self) {
        if self.pairing_offer.is_none() || self.remote_action_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.remote_action_rx = Some(rx);
        self.remote_notice = lang::pick("Refreshing pairing…", "正在刷新配对…").to_string();
        std::thread::spawn(move || {
            let client = crate::session_service::admin::LocalAdminClient::from_env();
            let result = tokio::runtime::Runtime::new()
                .map_err(|error| error.to_string())
                .and_then(|runtime| runtime.block_on(client.pending_pairings()).map_err(|error| error.to_string()));
            let _ = tx.send(RemoteActionResult::PairingsLoaded(result));
        });
    }

    fn confirm_pairing(&mut self) {
        if self.remote_action_rx.is_some() {
            return;
        }
        let Some(pairing) = self.pending_pairing.as_ref().filter(|pairing| pairing.device_name.is_some()) else {
            self.remote_notice = lang::pick("Waiting for the device; press R to refresh", "等待设备请求；按 R 刷新").to_string();
            return;
        };
        let id = pairing.id.clone();
        let (tx, rx) = mpsc::channel();
        self.remote_action_rx = Some(rx);
        self.remote_notice = lang::pick("Approving pairing…", "正在批准配对…").to_string();
        std::thread::spawn(move || {
            let client = crate::session_service::admin::LocalAdminClient::from_env();
            let result = tokio::runtime::Runtime::new()
                .map_err(|error| error.to_string())
                .and_then(|runtime| runtime.block_on(client.confirm_pairing(&id)).map_err(|error| error.to_string()));
            let _ = tx.send(RemoteActionResult::PairConfirmed(result));
        });
    }

    fn cancel_pairing(&mut self) {
        if self.remote_action_rx.is_some() {
            return;
        }
        let Some(offer) = self.pairing_offer.as_ref() else {
            return;
        };
        let id = offer.id.clone();
        let (tx, rx) = mpsc::channel();
        self.remote_action_rx = Some(rx);
        self.remote_notice = lang::pick("Cancelling pairing…", "正在取消配对…").to_string();
        std::thread::spawn(move || {
            let client = crate::session_service::admin::LocalAdminClient::from_env();
            let result = tokio::runtime::Runtime::new()
                .map_err(|error| error.to_string())
                .and_then(|runtime| runtime.block_on(client.cancel_pairing(&id)).map_err(|error| error.to_string()));
            let _ = tx.send(RemoteActionResult::PairCancelled(result));
        });
    }

    fn cycle_device(&mut self, forward: bool) {
        if self.devices.is_empty() {
            return;
        }
        self.selected_device = if forward {
            (self.selected_device + 1) % self.devices.len()
        } else if self.selected_device == 0 {
            self.devices.len() - 1
        } else {
            self.selected_device - 1
        };
        self.pending_revoke = false;
    }

    fn revoke_selected_device(&mut self) {
        let Some(device) = self.devices.get(self.selected_device) else {
            return;
        };
        if !self.pending_revoke {
            self.pending_revoke = true;
            return;
        }
        let token_id = device.token_id.clone();
        if crate::session_service::control::is_running() {
            let (tx, rx) = mpsc::channel();
            self.remote_action_rx = Some(rx);
            self.remote_notice = lang::pick("Revoking device…", "正在撤销设备…").to_string();
            std::thread::spawn(move || {
                let client = crate::session_service::admin::LocalAdminClient::from_env();
                let result = tokio::runtime::Runtime::new()
                    .map_err(|error| error.to_string())
                    .and_then(|runtime| runtime.block_on(client.revoke_device(&token_id)).map_err(|error| error.to_string()));
                let _ = tx.send(RemoteActionResult::DeviceRevoked(result));
            });
            self.pending_revoke = false;
            return;
        }
        let result = {
            let now = crate::session_service::remote::now_ms();
            self.mgr.db().revoke_backend_device(&token_id, now).map_err(anyhow::Error::from).and_then(|revoked| {
                anyhow::ensure!(revoked, "Active device not found");
                self.mgr.db().record_backend_audit("device.revoked", Some(&token_id), Some("tui"), now)?;
                Ok(())
            })
        };
        match result {
            Ok(()) => self.remote_notice = lang::pick("Device revoked", "设备已撤销").to_string(),
            Err(error) => self.remote_notice = format!("{}: {error}", lang::pick("Unable to revoke device", "无法撤销设备")),
        }
        self.pending_revoke = false;
        self.refresh_remote_state();
    }

    fn poll_remote_action(&mut self) {
        let result = self.remote_action_rx.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(RemoteActionResult::Failed("Background task stopped".into())),
        });
        let Some(result) = result else {
            return;
        };
        self.remote_action_rx = None;
        match result {
            RemoteActionResult::PairCreated(Ok(offer)) => {
                self.pairing_offer = Some(offer);
                self.pending_pairing = None;
                self.remote_notice.clear();
            }
            RemoteActionResult::PairCreated(Err(error)) => {
                self.remote_notice = format!("{}: {error}", lang::pick("Unable to create pairing", "无法创建配对"));
            }
            RemoteActionResult::PairingsLoaded(Ok(pairings)) => {
                if let Some(offer) = self.pairing_offer.as_ref() {
                    self.pending_pairing = pairings.into_iter().find(|pairing| pairing.id == offer.id);
                }
                self.remote_notice.clear();
            }
            RemoteActionResult::PairingsLoaded(Err(error)) => {
                self.remote_notice = format!("{}: {error}", lang::pick("Unable to refresh pairing", "无法刷新配对"));
            }
            RemoteActionResult::PairConfirmed(Ok(())) => {
                self.pairing_offer = None;
                self.pending_pairing = None;
                self.remote_notice = lang::pick("Pairing approved", "配对已批准").to_string();
                self.refresh_remote_state();
            }
            RemoteActionResult::PairConfirmed(Err(error)) => {
                self.remote_notice = format!("{}: {error}", lang::pick("Unable to approve pairing", "无法批准配对"));
            }
            RemoteActionResult::PairCancelled(Ok(())) => {
                self.pairing_offer = None;
                self.pending_pairing = None;
                self.remote_notice = lang::pick("Pairing cancelled", "配对已取消").to_string();
            }
            RemoteActionResult::PairCancelled(Err(error)) => {
                self.remote_notice = format!("{}: {error}", lang::pick("Unable to cancel pairing", "无法取消配对"));
            }
            RemoteActionResult::DeviceRevoked(Ok(())) => {
                self.remote_notice = lang::pick("Device revoked", "设备已撤销").to_string();
                self.refresh_remote_state();
            }
            RemoteActionResult::DeviceRevoked(Err(error)) => {
                self.remote_notice = format!("{}: {error}", lang::pick("Unable to revoke device", "无法撤销设备"));
            }
            RemoteActionResult::Failed(error) => {
                self.remote_notice = format!("{}: {error}", lang::pick("Remote action failed", "远程操作失败"));
            }
        }
    }
}

impl SettingsTab {
    fn render_pairing(&self, f: &mut Frame, area: Rect) {
        let Some(offer) = self.pairing_offer.as_ref() else {
            return;
        };
        let mut lines = vec![
            Line::styled(
                lang::pick("Scan with the AkironMux mobile app", "使用 AkironMux 移动端扫描"),
                Style::default().fg(theme::current().cyan),
            ),
            Line::from(""),
        ];
        match QrCode::new(offer.deep_link.as_bytes()) {
            Ok(code) => {
                let qr = code.render::<unicode::Dense1x2>().quiet_zone(true).build();
                lines.extend(qr.lines().map(|line| Line::raw(line.to_owned())));
            }
            Err(_) => lines.push(Line::raw(offer.deep_link.clone())),
        }
        lines.push(Line::from(""));
        if let Some(pairing) = self.pending_pairing.as_ref().filter(|pairing| pairing.device_name.is_some()) {
            lines.push(Line::styled(
                format!(
                    "{}: {} · {}: {}",
                    lang::pick("Device", "设备"),
                    pairing.device_name.as_deref().unwrap_or("-"),
                    lang::pick("Source", "来源"),
                    pairing.source.as_deref().unwrap_or("unknown")
                ),
                Style::default().fg(theme::current().purple),
            ));
            lines.push(Line::raw(lang::pick("Enter approve · R refresh · Esc cancel", "回车批准 · R 刷新 · Esc 取消")));
        } else {
            lines.push(Line::raw(lang::pick("Waiting for device · R refresh · Esc cancel", "等待设备 · R 刷新 · Esc 取消")));
        }
        if !self.remote_notice.is_empty() {
            lines.push(Line::styled(self.remote_notice.clone(), Style::default().fg(theme::current().yellow)));
        }
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::bordered()
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .title(lang::pick("Pair Remote device", "配对远程设备"))
                    .border_style(Style::default().fg(theme::current().cyan)),
            ),
            area,
        );
    }
}

impl TabContent for SettingsTab {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        self.poll_remote_action();
        let items = self.items();
        let refresh_label = lang::pick("Session Refresh", "会话刷新");
        let database_label = lang::pick("Database", "数据库");
        let remote_notice_label = lang::pick("Remote", "远程");
        let max_label_dw = items
            .iter()
            .map(|(label, _)| display_width(label))
            .chain([display_width(refresh_label), display_width(database_label), display_width(remote_notice_label)])
            .max()
            .unwrap_or(0);
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Length(5), Constraint::Length(13), Constraint::Min(5)])
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

        let backend = vec![
            setting_line(3, self.selected, items[3].0, &items[3].1, max_label_dw),
            Line::from(""),
            setting_line(4, self.selected, items[4].0, &items[4].1, max_label_dw),
            Line::from(""),
            setting_line(5, self.selected, items[5].0, &items[5].1, max_label_dw),
            Line::from(""),
            setting_line(6, self.selected, items[6].0, &items[6].1, max_label_dw),
            Line::from(""),
            setting_line(7, self.selected, items[7].0, &items[7].1, max_label_dw),
        ];
        f.render_widget(section(lang::pick("Services", "服务").into(), backend), sections[2]);

        let database = shorten_home(&config::db_path().display().to_string());
        let mut data = vec![
            readonly_line(refresh_label, lang::pick("Real-time", "实时"), max_label_dw),
            readonly_line(database_label, &database, max_label_dw),
        ];
        if !self.remote_notice.is_empty() {
            data.push(readonly_line(remote_notice_label, &self.remote_notice, max_label_dw));
        }
        f.render_widget(section(lang::pick("Data", "数据").into(), data), sections[3]);

        if self.pairing_offer.is_some() {
            self.render_pairing(f, area);
        }
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        if self.pairing_offer.is_some() {
            match code {
                KeyCode::Esc => {
                    self.cancel_pairing();
                }
                KeyCode::Char('r' | 'R') => self.refresh_pairing(),
                KeyCode::Enter => self.confirm_pairing(),
                _ => {}
            }
            return true;
        }
        match code {
            KeyCode::Tab | KeyCode::BackTab => return false,
            KeyCode::Char('j') | KeyCode::Down => {
                let max = self.items().len().saturating_sub(1);
                self.selected = (self.selected + 1).min(max);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Char('l' | 'L') | KeyCode::Right => match self.selected {
                0 => self.cycle_theme(true),
                1 => self.cycle_lang(true),
                2 => self.cycle_mode(true),
                3 => self.toggle_session_service(),
                4 => self.toggle_remote_backend(),
                5 => self.toggle_remote_backend(),
                6 => self.create_pairing(),
                7 => self.cycle_device(true),
                _ => {}
            },
            KeyCode::Char('h' | 'H') | KeyCode::Left => match self.selected {
                0 => self.cycle_theme(false),
                1 => self.cycle_lang(false),
                2 => self.cycle_mode(false),
                3 => self.toggle_session_service(),
                4 => self.toggle_remote_backend(),
                5 => self.toggle_remote_backend(),
                6 => self.create_pairing(),
                7 => self.cycle_device(false),
                _ => {}
            },
            KeyCode::Enter => match self.selected {
                4 | 5 => self.toggle_remote_backend(),
                6 => self.create_pairing(),
                _ => return false,
            },
            KeyCode::Char('x' | 'X') if self.selected == 7 => self.revoke_selected_device(),
            _ => return false,
        }
        true
    }

    fn shortcut_groups(&self) -> Vec<Vec<(String, Color)>> {
        let l = lang::current();
        vec![
            vec![(" J/K ".into(), theme::current().comment), (l.sc_nav.into(), theme::current().comment)],
            vec![(" H/L ".into(), theme::current().comment), (l.sc_toggle.into(), theme::current().comment)],
            vec![
                (" Enter ".into(), theme::current().comment),
                (lang::pick("action", "操作").into(), theme::current().comment),
            ],
            vec![(" X X ".into(), theme::current().comment), (lang::pick("revoke", "撤销").into(), theme::current().comment)],
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
