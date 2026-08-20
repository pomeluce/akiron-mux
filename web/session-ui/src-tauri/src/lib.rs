use std::sync::Mutex;
#[cfg(desktop)]
use tauri::{Manager, WindowEvent};
#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{KillTimer, SetTimer, WM_DWMCOLORIZATIONCOLORCHANGED, WM_NCDESTROY, WM_SETTINGCHANGE, WM_THEMECHANGED, WM_TIMER},
    },
};

#[cfg(target_os = "windows")]
const NATIVE_BACKDROP_SUBCLASS_ID: usize = 0x414b_4d58;
#[cfg(target_os = "windows")]
const NATIVE_BACKDROP_TIMER_ID: usize = 0x414b_4d59;
#[cfg(target_os = "windows")]
const NATIVE_BACKDROP_DEBOUNCE_MS: u32 = 100;

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Clone, Copy)]
struct NativeBackdropSettings {
    dark: bool,
    material_transparency: u8,
}

#[derive(Default)]
struct NativeBackdropState(Mutex<Option<NativeBackdropSettings>>);

#[cfg(any(target_os = "windows", test))]
fn native_theme(settings: NativeBackdropSettings) -> tauri::Theme {
    if settings.dark {
        tauri::Theme::Dark
    } else {
        tauri::Theme::Light
    }
}

fn apply_native_backdrop(window: &tauri::WebviewWindow, settings: NativeBackdropSettings) {
    #[cfg(target_os = "windows")]
    {
        let _ = window.set_theme(Some(native_theme(settings)));
        let transparency = settings.material_transparency.min(100) as u16;
        let tint_alpha = if transparency <= 30 {
            (255 - transparency * 246 / 30) as u8
        } else {
            (9 - (transparency - 30) * 8 / 70) as u8
        };
        let acrylic_tint = if settings.dark { (8, 10, 9, tint_alpha) } else { (238, 240, 239, tint_alpha) };
        if window_vibrancy::apply_mica(window, Some(settings.dark)).is_err() {
            let _ = window_vibrancy::apply_acrylic(window, Some(acrylic_tint));
        }
    }

    #[cfg(not(target_os = "windows"))]
    let _ = (window, settings);
}

#[cfg(desktop)]
fn restore_native_backdrop(app: &tauri::AppHandle) {
    let settings = app.state::<NativeBackdropState>().0.lock().ok().and_then(|state| *state);
    if let (Some(settings), Some(webview)) = (settings, app.get_webview_window("main")) {
        apply_native_backdrop(&webview, settings);
    }
}

#[cfg(target_os = "windows")]
struct NativeBackdropHook {
    app: tauri::AppHandle,
}

#[cfg(target_os = "windows")]
fn invalidates_native_backdrop(message: u32) -> bool {
    matches!(message, WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DWMCOLORIZATIONCOLORCHANGED)
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn native_backdrop_window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _subclass_id: usize, hook_data: usize) -> LRESULT {
    if invalidates_native_backdrop(message) && hook_data != 0 {
        // DWM recalculates the backdrop during default handling, so restore only after the
        // complete burst of native appearance messages has settled.
        unsafe { SetTimer(hwnd, NATIVE_BACKDROP_TIMER_ID, NATIVE_BACKDROP_DEBOUNCE_MS, None) };
    }
    if message == WM_TIMER && wparam == NATIVE_BACKDROP_TIMER_ID && hook_data != 0 {
        unsafe { KillTimer(hwnd, NATIVE_BACKDROP_TIMER_ID) };
        let hook = unsafe { &*(hook_data as *const NativeBackdropHook) };
        restore_native_backdrop(&hook.app);
        return 0;
    }
    if message == WM_NCDESTROY && hook_data != 0 {
        unsafe {
            KillTimer(hwnd, NATIVE_BACKDROP_TIMER_ID);
            RemoveWindowSubclass(hwnd, Some(native_backdrop_window_proc), NATIVE_BACKDROP_SUBCLASS_ID);
            drop(Box::from_raw(hook_data as *mut NativeBackdropHook));
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[cfg(target_os = "windows")]
fn setup_native_backdrop_listener(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let window = app.get_webview_window("main").ok_or("Main window is unavailable")?;
    let hwnd = window.hwnd()?.0 as HWND;
    let hook_data = Box::into_raw(Box::new(NativeBackdropHook { app: app.handle().clone() })) as usize;
    let installed = unsafe { SetWindowSubclass(hwnd, Some(native_backdrop_window_proc), NATIVE_BACKDROP_SUBCLASS_ID, hook_data) };
    if installed == 0 {
        unsafe { drop(Box::from_raw(hook_data as *mut NativeBackdropHook)) };
        return Err("Failed to observe Windows backdrop setting changes".into());
    }
    Ok(())
}

#[cfg(all(desktop, not(target_os = "windows")))]
fn setup_native_backdrop_listener(_app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[tauri::command]
fn sync_native_backdrop(window: tauri::WebviewWindow, state: tauri::State<'_, NativeBackdropState>, dark: bool, material_transparency: u8) {
    let settings = NativeBackdropSettings { dark, material_transparency };
    if let Ok(mut current) = state.0.lock() {
        *current = Some(settings);
    }
    apply_native_backdrop(&window, settings);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().manage(NativeBackdropState::default());
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .manage(tray::TrayState::default())
        .setup(|app| {
            tray::setup(app)?;
            setup_native_backdrop_listener(app)
        })
        .on_window_event(|window, event| {
            tray::handle_window_event(window, event);
            if matches!(event, WindowEvent::Focused(true) | WindowEvent::ThemeChanged(_)) {
                restore_native_backdrop(window.app_handle());
            }
        });
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            sync_native_backdrop,
            #[cfg(desktop)]
            tray::sync_tray_state,
            backend::list_backend_profiles,
            backend::save_backend_profile,
            backend::pair_backend_profile,
            backend::activate_backend_profile,
            backend::reorder_backend_profiles,
            backend::delete_backend_profile,
            backend::test_backend_profile,
            backend::refresh_backend_profile,
            backend::backend_request
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AkironMux desktop application");
}

#[cfg(test)]
mod tests {
    use super::{native_theme, NativeBackdropSettings};

    #[test]
    fn native_theme_follows_the_resolved_application_theme() {
        let dark = NativeBackdropSettings {
            dark: true,
            material_transparency: 30,
        };
        let light = NativeBackdropSettings {
            dark: false,
            material_transparency: 30,
        };

        assert_eq!(native_theme(dark), tauri::Theme::Dark);
        assert_eq!(native_theme(light), tauri::Theme::Light);
    }
}
mod backend;
#[cfg(desktop)]
mod tray;
