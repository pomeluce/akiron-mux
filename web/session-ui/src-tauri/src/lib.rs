#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().manage(native_appearance::NativeAppearanceState::default());
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .manage(tray::TrayState::default())
        .setup(|app| {
            tray::setup(app)?;
            native_appearance::install(app)
        })
        .on_window_event(|window, event| {
            tray::handle_window_event(window, event);
            native_appearance::handle_window_event(window, event);
        });
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            native_appearance::sync_native_backdrop,
            #[cfg(desktop)]
            tray::sync_tray_state,
            backend::list_backend_profiles,
            backend::apply_backend_profile_intent,
            backend::test_backend_profile,
            backend::backend_request
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AkironMux desktop application");
}

mod backend;
mod native_appearance;
#[cfg(desktop)]
mod tray;
