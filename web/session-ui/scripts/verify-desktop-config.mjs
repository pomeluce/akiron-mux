import assert from 'node:assert/strict';
import fs from 'node:fs';

const config = JSON.parse(fs.readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const capability = JSON.parse(fs.readFileSync(new URL('../src-tauri/capabilities/main-window.json', import.meta.url), 'utf8'));
const cargoManifest = fs.readFileSync(new URL('../src-tauri/Cargo.toml', import.meta.url), 'utf8');
const desktopLibrary = fs.readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8');
const nativeAppearance = fs.readFileSync(new URL('../src-tauri/src/native_appearance/mod.rs', import.meta.url), 'utf8');
const windowsAppearance = fs.readFileSync(new URL('../src-tauri/src/native_appearance/windows.rs', import.meta.url), 'utf8');
const packageManifest = fs.readFileSync(new URL('../package.json', import.meta.url), 'utf8');
const backendHook = fs.readFileSync(new URL('../src/features/backends/use-backends.ts', import.meta.url), 'utf8');
const backendLifecycle = fs.readFileSync(new URL('../src/features/backends/backend-profile-lifecycle.ts', import.meta.url), 'utf8');
const app = fs.readFileSync(new URL('../src/app/app.tsx', import.meta.url), 'utf8');
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
assert.match(desktopLibrary, /mod native_appearance;/, 'native appearance behavior must live behind a dedicated module');
assert.doesNotMatch(desktopLibrary, /windows_sys|window_vibrancy|WM_[A-Z_]+/, 'the desktop entry point must not own platform appearance details');
assert.match(desktopLibrary, /manage\(native_appearance::NativeAppearanceState::default\(\)\)/, 'the desktop shell must retain resolved native appearance state');
assert.match(desktopLibrary, /native_appearance::install/, 'the desktop shell must install native appearance handling during setup');
assert.match(desktopLibrary, /native_appearance::handle_window_event/, 'window events must cross the native appearance seam');
assert.match(desktopLibrary, /native_appearance::sync_native_backdrop/, 'native appearance synchronization must be registered as a Tauri command');
assert.match(nativeAppearance, /#\[cfg\(target_os = "windows"\)\]\s*mod windows;/, 'Windows appearance code must be isolated behind target compilation');
assert.match(nativeAppearance, /#\[cfg\(not\(target_os = "windows"\)\)\]/, 'non-Windows builds must use the platform-neutral no-op path');
assert.match(windowsAppearance, /use windows_sys::Win32/, 'the Windows adapter must own native appearance messages');
assert.match(windowsAppearance, /window_vibrancy::/, 'the Windows adapter must own native backdrop materials');
assert.match(desktopLibrary, /backend::list_backend_profiles/, 'desktop backend profile commands must be registered with Tauri');
assert.match(desktopLibrary, /backend::apply_backend_profile_intent/, 'Backend Profile lifecycle intents must be handled by the native desktop layer');
assert.match(backendLifecycle, /apply_backend_profile_intent/, 'the WebView must cross one typed Backend Profile lifecycle seam');
assert.match(backendLifecycle, /generation !== this\.generation/, 'late refresh results from a previous Backend Profile must be ignored');
assert.match(backendLifecycle, /this\.inFlight/, 'active Remote Profile refreshes must not overlap');
assert.doesNotMatch(app, /test_backend_profile|BACKEND_IDENTITY_CHANGED/, 'App must not orchestrate Backend Profile security ordering');
assert.doesNotMatch(settingsDialog, /BACKEND_IDENTITY_CHANGED/, 'settings must consume typed identity confirmation outcomes');
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
