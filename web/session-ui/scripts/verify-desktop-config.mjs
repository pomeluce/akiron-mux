import assert from 'node:assert/strict';
import fs from 'node:fs';

const config = JSON.parse(fs.readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const capability = JSON.parse(fs.readFileSync(new URL('../src-tauri/capabilities/main-window.json', import.meta.url), 'utf8'));
const cargoManifest = fs.readFileSync(new URL('../src-tauri/Cargo.toml', import.meta.url), 'utf8');
const desktopLibrary = fs.readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8');
const packageManifest = fs.readFileSync(new URL('../package.json', import.meta.url), 'utf8');
const backendHook = fs.readFileSync(new URL('../src/features/backends/use-backends.ts', import.meta.url), 'utf8');
const settingsDialog = fs.readFileSync(new URL('../src/features/preferences/settings-dialog.tsx', import.meta.url), 'utf8');
const terminalView = fs.readFileSync(new URL('../src/features/sessions/terminal-view.tsx', import.meta.url), 'utf8');
const releaseWorkflow = fs.readFileSync(new URL('../../../.github/workflows/release.yml', import.meta.url), 'utf8');
const window = config.app.windows.find(candidate => candidate.label === 'main');
const nsis = config.bundle.windows.nsis;
const windowsGuiJob = releaseWorkflow.slice(releaseWorkflow.indexOf('  gui-windows:'), releaseWorkflow.indexOf('\n  publish:'));
const windowsFrontendBuild = windowsGuiJob.indexOf('pnpm build');
const windowsCredentialTest = windowsGuiJob.indexOf('Test Windows credential integration');

assert.equal(nsis.installMode, 'perMachine', 'Windows installer must target Program Files');
assert.equal(nsis.installerIcon, 'icons/icon.ico', 'NSIS installer must use the application icon');
assert.equal(window.transparent, true, 'native backdrop materials require a transparent Tauri window');
assert.equal(window.decorations, false, 'desktop builds must use the custom title bar');
assert.match(cargoManifest, /\[target\.'cfg\(target_os = "windows"\)'\.dependencies\][\s\S]*window-vibrancy\s*=/, 'Windows backdrop materials must use the native vibrancy API');
assert.match(desktopLibrary, /#\[cfg\(target_os = "windows"\)\]\s*use windows_sys::Win32/, 'Windows appearance messages must not compile on macOS or Linux');
assert.match(
  desktopLibrary,
  /#\[cfg\(all\(desktop, not\(target_os = "windows"\)\)\)\]\s*fn setup_native_backdrop_listener[\s\S]*?\{\s*Ok\(\(\)\)\s*\}/,
  'macOS and Linux must use the no-op native backdrop listener',
);
assert.match(
  desktopLibrary,
  /#\[cfg\(not\(target_os = "windows"\)\)\]\s*let _ = \(window, settings\);/,
  'macOS and Linux must not apply Windows native backdrop effects',
);
assert.match(desktopLibrary, /window_vibrancy::apply_mica\([^,]+, Some\(settings\.dark\)\)/, 'Windows 11 Mica must follow the resolved application theme');
assert.match(desktopLibrary, /apply_mica[\s\S]*is_err\(\)[\s\S]*window_vibrancy::apply_acrylic/, 'Windows 10 must fall back to a theme-aware Acrylic tint when Mica is unavailable');
assert.match(desktopLibrary, /material_transparency[\s\S]*tint_alpha/, 'Windows 10 Acrylic tint must follow material transparency');
assert.match(desktopLibrary, /sync_native_backdrop/, 'the desktop shell must expose native backdrop theme synchronization');
assert.match(desktopLibrary, /NativeBackdropState/, 'the desktop shell must retain the resolved native backdrop state');
assert.match(
  desktopLibrary,
  /WindowEvent::Focused\(true\)[\s\S]*restore_native_backdrop/,
  'Windows must restore the native backdrop after system settings invalidate it while the app is unfocused',
);
assert.match(desktopLibrary, /SetWindowSubclass/, 'Windows must observe native setting broadcasts while the app is unfocused');
assert.match(
  desktopLibrary,
  /WM_SETTINGCHANGE[\s\S]*WM_THEMECHANGED[\s\S]*WM_DWMCOLORIZATIONCOLORCHANGED/,
  'Windows setting, theme, and DWM color broadcasts must schedule a native backdrop restore',
);
assert.match(desktopLibrary, /SetTimer/, 'Windows backdrop restoration must wait until native setting broadcasts settle');
assert.match(
  desktopLibrary,
  /WM_TIMER[\s\S]*KillTimer[\s\S]*restore_native_backdrop/,
  'Windows must restore the native backdrop after the debounce timer fires',
);
assert.match(desktopLibrary, /generate_handler!\[\s*sync_native_backdrop(?:,|\s*\])/, 'native backdrop synchronization must be registered as a Tauri command');
assert.match(desktopLibrary, /backend::list_backend_profiles/, 'desktop backend profile commands must be registered with Tauri');
assert.match(desktopLibrary, /backend::pair_backend_profile/, 'device pairing must be handled by the native desktop layer');
assert.match(cargoManifest, /tauri\s*=\s*\{[^}]*features\s*=\s*\["tray-icon"\]/, 'desktop builds must enable the Tauri tray API');
assert.match(desktopLibrary, /tray::setup/, 'desktop builds must create the system tray');
assert.match(desktopLibrary, /tray::sync_tray_state/, 'the WebView must be able to synchronize tray preferences and sessions');
assert.match(cargoManifest, /tauri-plugin-single-instance\s*=/, 'desktop builds must prevent duplicate application and tray instances');
assert.match(desktopLibrary, /tauri_plugin_single_instance::init/, 'a second application launch must restore the existing main window');
assert.ok(
  desktopLibrary.indexOf('tauri_plugin_single_instance::init') < desktopLibrary.indexOf('tray::setup'),
  'single-instance protection must initialize before the tray is created',
);
assert.match(packageManifest, /"@tauri-apps\/plugin-opener"\s*:/, 'terminal links must use the native external URL opener');
assert.match(cargoManifest, /tauri-plugin-opener\s*=/, 'desktop builds must include the native external URL opener');
assert.match(desktopLibrary, /tauri_plugin_opener::init\(\)/, 'the native external URL opener must be registered with Tauri');
assert.match(terminalView, /WebLinksAddon/, 'plain HTTP links in terminal output must be detected');
assert.match(terminalView, /import \{ openUrl \} from '@tauri-apps\/plugin-opener'/, 'terminal links must use the native external URL opener');
assert.match(terminalView, /linkHandler[\s\S]*openExternalUrl/, 'OSC 8 terminal links must use the external URL handler');
assert.doesNotMatch(backendHook, /\btoken\b/i, 'long-lived device tokens must not cross the WebView backend hook');
assert.doesNotMatch(settingsDialog, /\btoken\b/i, 'long-lived device tokens must not enter WebView settings state');
assert.ok(capability.permissions.includes('core:window:allow-close'));
assert.ok(capability.permissions.includes('core:window:allow-minimize'));
assert.ok(capability.permissions.includes('core:window:allow-toggle-maximize'));
assert.ok(capability.permissions.includes('notification:default'), 'desktop notifications must be permitted');
assert.ok(capability.permissions.includes('opener:allow-default-urls'), 'only default external URL schemes may leave the WebView');
assert.match(cargoManifest, /tauri-plugin-notification\s*=/, 'notification plugin must be a desktop dependency');
assert.match(desktopLibrary, /tauri_plugin_notification::init\(\)/, 'notification plugin must be registered with Tauri');
assert.ok(
  windowsFrontendBuild >= 0 && windowsCredentialTest >= 0 && windowsFrontendBuild < windowsCredentialTest,
  'Windows credential tests must build frontendDist before compiling the Tauri context',
);

console.log('Desktop packaging configuration is valid.');
