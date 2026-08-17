use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, Window, WindowEvent,
};

const TRAY_ID: &str = "akmux-tray";

pub struct TrayState {
    close_to_tray: AtomicBool,
}

impl Default for TrayState {
    fn default() -> Self {
        Self {
            close_to_tray: AtomicBool::new(true),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraySession {
    id: String,
    title: String,
    agent: String,
}

pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let menu = build_menu(app.handle(), "en", &[])?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("AkironMux")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if id == "tray-open" {
                show_main_window(app);
            } else if id == "tray-quit" {
                app.exit(0);
            } else if let Some(session_id) = id.strip_prefix("tray-session:") {
                show_main_window(app);
                let _ = app.emit("tray-open-session", session_id.to_string());
            }
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        let close_to_tray = window.state::<TrayState>().close_to_tray.load(Ordering::Acquire);
        if close_to_tray {
            api.prevent_close();
            let _ = window.hide();
        }
    }
}

#[tauri::command]
pub fn sync_tray_state(app: AppHandle, state: State<'_, TrayState>, close_to_tray: bool, locale: String, sessions: Vec<TraySession>) -> Result<(), String> {
    state.close_to_tray.store(close_to_tray, Ordering::Release);
    let menu = build_menu(&app, &locale, &sessions).map_err(|error| error.to_string())?;
    let tray = app.tray_by_id(TRAY_ID).ok_or_else(|| "System tray is unavailable".to_string())?;
    tray.set_menu(Some(menu)).map_err(|error| error.to_string())
}

fn build_menu<R: tauri::Runtime>(app: &AppHandle<R>, locale: &str, sessions: &[TraySession]) -> tauri::Result<tauri::menu::Menu<R>> {
    let chinese = locale == "zh-CN";
    let mut menu = MenuBuilder::new(app).text("tray-open", if chinese { "打开 AkironMux" } else { "Open AkironMux" });
    if !sessions.is_empty() {
        menu = menu.separator();
        for session in sessions {
            let agent = if session.agent == "claude" { "Claude Code" } else { "Codex" };
            menu = menu.text(format!("tray-session:{}", session.id), format!("{} · {}", agent, menu_title(&session.title)));
        }
    }
    menu.separator().text("tray-quit", if chinese { "退出程序" } else { "Quit" }).build()
}

fn menu_title(title: &str) -> String {
    let clean = title.replace(['\r', '\n', '\t'], " ");
    let mut chars = clean.trim().chars();
    let value = chars.by_ref().take(56).collect::<String>();
    if chars.next().is_some() {
        format!("{value}…")
    } else if value.is_empty() {
        "Session".to_string()
    } else {
        value
    }
}

fn show_main_window<R: tauri::Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::menu_title;

    #[test]
    fn tray_session_titles_are_single_line_and_bounded() {
        let title = menu_title("first\nsecond\twith a title that is deliberately much longer than the tray menu should display in full");
        assert!(!title.contains(['\n', '\t']));
        assert!(title.chars().count() <= 57);
    }
}
