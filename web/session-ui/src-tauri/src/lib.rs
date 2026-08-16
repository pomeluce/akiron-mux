#[tauri::command]
fn sync_native_backdrop(window: tauri::WebviewWindow, dark: bool, material_transparency: u8) {
    #[cfg(target_os = "windows")]
    {
        let transparency = material_transparency.min(100) as u16;
        let tint_alpha = (1 + ((100 - transparency) * 199 / 100)) as u8;
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
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            sync_native_backdrop,
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
