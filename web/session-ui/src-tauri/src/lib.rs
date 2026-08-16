#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            #[cfg(target_os = "windows")]
            {
                use tauri::Manager;

                if let Some(window) = app.get_webview_window("main") {
                    // Mica uses the desktop wallpaper instead of sampling windows behind the app.
                    // Acrylic remains the compatibility fallback for Windows 10.
                    if window_vibrancy::apply_mica(&window, None).is_err() {
                        let _ = window_vibrancy::apply_acrylic(&window, None);
                    }
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run AkironMux desktop application");
}
