#[tauri::command]
fn sync_native_backdrop(window: tauri::WebviewWindow, dark: bool, material_transparency: u8) {
    #[cfg(target_os = "windows")]
    {
        let transparency = material_transparency.min(100) as u16;
        let tint_alpha = if transparency <= 30 {
            (255 - transparency * 246 / 30) as u8
        } else {
            (9 - (transparency - 30) * 8 / 70) as u8
        };
        let acrylic_tint = if dark { (8, 10, 9, tint_alpha) } else { (238, 240, 239, tint_alpha) };
        if window_vibrancy::apply_mica(&window, Some(dark)).is_err() {
            let _ = window_vibrancy::apply_acrylic(&window, Some(acrylic_tint));
        }
    }

    #[cfg(not(target_os = "windows"))]
    let _ = (window, dark, material_transparency);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .manage(tray::TrayState::default())
        .setup(tray::setup)
        .on_window_event(tray::handle_window_event);
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
