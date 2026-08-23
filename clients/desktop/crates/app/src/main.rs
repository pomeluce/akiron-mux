mod workspace;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use akmux_desktop_platform::{DesktopTray, InstanceOutcome, SingleInstance};
use gpui::*;
use gpui_component::{Root, TitleBar};
use gpui_component_assets::Assets;
use workspace::DesktopWorkspace;

fn main() {
    tracing_subscriber::fmt::init();
    let instance = match SingleInstance::acquire() {
        Ok(InstanceOutcome::Primary(instance)) => instance,
        Ok(InstanceOutcome::Secondary) => return,
        Err(error) => {
            eprintln!("{error}");
            return;
        }
    };
    let app = gpui_platform::application().with_assets(Assets);
    app.run(move |cx| {
        cx.set_app_identity("dev.akiron.mux", "AkironMux");
        gpui_component::init(cx);
        let (tray, tray_events) = DesktopTray::new().expect("failed to create the desktop tray");
        let close_to_tray = Arc::new(AtomicBool::new(true));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(820.)), cx)),
            window_min_size: Some(size(px(900.), px(600.))),
            ..TitleBar::window_options()
        };
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                let should_hide = close_to_tray.clone();
                window.on_window_should_close(cx, move |_, cx| {
                    if should_hide.load(Ordering::Relaxed) {
                        cx.hide();
                        false
                    } else {
                        true
                    }
                });
                let view = cx.new(|cx| DesktopWorkspace::new(window, tray, tray_events, instance, close_to_tray, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open AkironMux Desktop window");
            write_startup_marker();
        })
        .detach();
    });
}

fn write_startup_marker() {
    let Ok(path) = std::env::var("AKMUX_STARTUP_MARKER") else {
        return;
    };
    let _ = std::fs::write(path, b"ready\n");
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    #[test]
    fn nsis_installer_uses_the_app_icon_and_supports_program_files() {
        let desktop_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let config: serde_json::Value = serde_json::from_slice(&fs::read(desktop_dir.join("packager.json")).unwrap()).unwrap();
        let icon = config["nsis"]["installerIcon"].as_str().expect("NSIS installer icon must be configured");

        assert_eq!(icon, "icons/icon.ico");
        assert!(desktop_dir.join(icon).is_file());
        assert_eq!(config["nsis"]["installMode"], "both");
    }
}
