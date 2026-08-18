use std::sync::Mutex;
#[cfg(desktop)]
use tauri::{Manager, WindowEvent};

#[derive(Clone, Copy)]
struct NativeBackdropSettings {
    dark: bool,
    material_transparency: u8,
}

#[derive(Default)]
struct NativeBackdropState(Mutex<Option<NativeBackdropSettings>>);

fn apply_native_backdrop(window: &tauri::WebviewWindow, settings: NativeBackdropSettings) {
    #[cfg(target_os = "windows")]
    {
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
fn restore_native_backdrop(window: &tauri::Window) {
    let settings = window.state::<NativeBackdropState>().0.lock().ok().and_then(|state| *state);
    if let (Some(settings), Some(webview)) = (settings, window.app_handle().get_webview_window(window.label())) {
        apply_native_backdrop(&webview, settings);
    }
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
        .setup(tray::setup)
        .on_window_event(|window, event| {
            tray::handle_window_event(window, event);
            if matches!(event, WindowEvent::Focused(true) | WindowEvent::ThemeChanged(_)) {
                restore_native_backdrop(window);
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
mod backend;
#[cfg(desktop)]
mod tray;
