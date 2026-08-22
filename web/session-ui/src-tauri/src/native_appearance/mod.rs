use std::sync::Mutex;

#[cfg(target_os = "windows")]
mod windows;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AppearanceSettings {
    dark: bool,
    material_transparency: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResolvedAppearance {
    theme: tauri::Theme,
    acrylic_tint: (u8, u8, u8, u8),
}

#[derive(Default)]
pub(crate) struct NativeAppearanceState(Mutex<Option<AppearanceSettings>>);

impl NativeAppearanceState {
    fn remember(&self, settings: AppearanceSettings) {
        if let Ok(mut current) = self.0.lock() {
            *current = Some(settings);
        }
    }

    fn current(&self) -> Option<AppearanceSettings> {
        self.0.lock().ok().and_then(|current| *current)
    }
}

fn resolve(settings: AppearanceSettings) -> ResolvedAppearance {
    let transparency = settings.material_transparency.min(100) as u16;
    let tint_alpha = if transparency <= 30 {
        (255 - transparency * 246 / 30) as u8
    } else {
        (9 - (transparency - 30) * 8 / 70) as u8
    };
    let (red, green, blue) = if settings.dark { (8, 10, 9) } else { (238, 240, 239) };

    ResolvedAppearance {
        theme: if settings.dark { tauri::Theme::Dark } else { tauri::Theme::Light },
        acrylic_tint: (red, green, blue, tint_alpha),
    }
}

fn apply(window: &tauri::WebviewWindow, settings: AppearanceSettings) {
    let appearance = resolve(settings);

    #[cfg(target_os = "windows")]
    windows::apply(window, settings, appearance);

    #[cfg(not(target_os = "windows"))]
    let _ = (window, settings, appearance);
}

#[cfg(desktop)]
pub(crate) fn install(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    windows::install(app)?;

    #[cfg(not(target_os = "windows"))]
    let _ = app;

    Ok(())
}

#[cfg(desktop)]
pub(crate) fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    use tauri::Manager;

    if matches!(event, tauri::WindowEvent::Focused(true) | tauri::WindowEvent::ThemeChanged(_)) {
        restore(window.app_handle());
    }
}

#[cfg(desktop)]
fn restore(app: &tauri::AppHandle) {
    use tauri::Manager;

    let settings = app.state::<NativeAppearanceState>().current();
    if let (Some(settings), Some(window)) = (settings, app.get_webview_window("main")) {
        apply(&window, settings);
    }
}

#[tauri::command]
pub(crate) fn sync_native_backdrop(window: tauri::WebviewWindow, state: tauri::State<'_, NativeAppearanceState>, dark: bool, material_transparency: u8) {
    let settings = AppearanceSettings { dark, material_transparency };
    state.remember(settings);
    apply(&window, settings);
}

#[cfg(test)]
mod tests {
    use super::{resolve, AppearanceSettings, NativeAppearanceState};

    #[test]
    fn resolves_application_theme_and_tint_palette() {
        let dark = resolve(AppearanceSettings {
            dark: true,
            material_transparency: 30,
        });
        let light = resolve(AppearanceSettings {
            dark: false,
            material_transparency: 30,
        });

        assert_eq!(dark.theme, tauri::Theme::Dark);
        assert_eq!(dark.acrylic_tint, (8, 10, 9, 9));
        assert_eq!(light.theme, tauri::Theme::Light);
        assert_eq!(light.acrylic_tint, (238, 240, 239, 9));
    }

    #[test]
    fn clamps_transparency_before_resolving_acrylic_alpha() {
        let opaque = resolve(AppearanceSettings {
            dark: true,
            material_transparency: 0,
        });
        let transparent = resolve(AppearanceSettings {
            dark: true,
            material_transparency: 100,
        });
        let above_range = resolve(AppearanceSettings {
            dark: true,
            material_transparency: u8::MAX,
        });

        assert_eq!(opaque.acrylic_tint.3, 255);
        assert_eq!(transparent.acrylic_tint.3, 1);
        assert_eq!(above_range.acrylic_tint.3, 1);
    }

    #[test]
    fn remembers_the_last_resolved_settings_for_restoration() {
        let state = NativeAppearanceState::default();
        let first = AppearanceSettings {
            dark: false,
            material_transparency: 10,
        };
        let latest = AppearanceSettings {
            dark: true,
            material_transparency: 75,
        };

        assert_eq!(state.current(), None);
        state.remember(first);
        state.remember(latest);
        assert_eq!(state.current(), Some(latest));
    }
}
