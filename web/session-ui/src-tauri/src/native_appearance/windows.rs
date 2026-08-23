use super::{restore, AppearanceSettings, ResolvedAppearance};
use tauri::Manager;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{KillTimer, SetTimer, WM_DWMCOLORIZATIONCOLORCHANGED, WM_NCDESTROY, WM_SETTINGCHANGE, WM_THEMECHANGED, WM_TIMER},
    },
};

const SUBCLASS_ID: usize = 0x414b_4d58;
const TIMER_ID: usize = 0x414b_4d59;
const RESTORE_DEBOUNCE_MS: u32 = 100;

pub(super) fn apply(window: &tauri::WebviewWindow, settings: AppearanceSettings, appearance: ResolvedAppearance) {
    let _ = window.set_theme(Some(appearance.theme));
    if window_vibrancy::apply_mica(window, Some(settings.dark)).is_err() {
        let _ = window_vibrancy::apply_acrylic(window, Some(appearance.acrylic_tint));
    }
}

pub(super) fn install(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let window = app.get_webview_window("main").ok_or("Main window is unavailable")?;
    let hwnd = window.hwnd()?.0 as HWND;
    let hook_data = Box::into_raw(Box::new(AppearanceHook { app: app.handle().clone() })) as usize;
    let installed = unsafe { SetWindowSubclass(hwnd, Some(appearance_window_proc), SUBCLASS_ID, hook_data) };
    if installed == 0 {
        unsafe { drop(Box::from_raw(hook_data as *mut AppearanceHook)) };
        return Err("Failed to observe Windows appearance setting changes".into());
    }
    Ok(())
}

struct AppearanceHook {
    app: tauri::AppHandle,
}

fn invalidates_appearance(message: u32) -> bool {
    matches!(message, WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DWMCOLORIZATIONCOLORCHANGED)
}

unsafe extern "system" fn appearance_window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _subclass_id: usize, hook_data: usize) -> LRESULT {
    if invalidates_appearance(message) && hook_data != 0 {
        // DWM recalculates the backdrop during default handling, so restore only after the
        // complete burst of native appearance messages has settled.
        unsafe { SetTimer(hwnd, TIMER_ID, RESTORE_DEBOUNCE_MS, None) };
    }
    if message == WM_TIMER && wparam == TIMER_ID && hook_data != 0 {
        unsafe { KillTimer(hwnd, TIMER_ID) };
        let hook = unsafe { &*(hook_data as *const AppearanceHook) };
        restore(&hook.app);
        return 0;
    }
    if message == WM_NCDESTROY && hook_data != 0 {
        unsafe {
            KillTimer(hwnd, TIMER_ID);
            RemoveWindowSubclass(hwnd, Some(appearance_window_proc), SUBCLASS_ID);
            drop(Box::from_raw(hook_data as *mut AppearanceHook));
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::invalidates_appearance;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_DWMCOLORIZATIONCOLORCHANGED, WM_SETTINGCHANGE, WM_THEMECHANGED, WM_TIMER};

    #[test]
    fn restores_only_after_native_appearance_invalidations() {
        assert!(invalidates_appearance(WM_SETTINGCHANGE));
        assert!(invalidates_appearance(WM_THEMECHANGED));
        assert!(invalidates_appearance(WM_DWMCOLORIZATIONCOLORCHANGED));
        assert!(!invalidates_appearance(WM_TIMER));
    }
}
