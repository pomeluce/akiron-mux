import assert from 'node:assert/strict';
import fs from 'node:fs';

const config = JSON.parse(fs.readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const capability = JSON.parse(fs.readFileSync(new URL('../src-tauri/capabilities/main-window.json', import.meta.url), 'utf8'));
const cargoManifest = fs.readFileSync(new URL('../src-tauri/Cargo.toml', import.meta.url), 'utf8');
const desktopLibrary = fs.readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8');
const backendHook = fs.readFileSync(new URL('../src/features/backends/use-backends.ts', import.meta.url), 'utf8');
const settingsDialog = fs.readFileSync(new URL('../src/features/preferences/settings-dialog.tsx', import.meta.url), 'utf8');
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
assert.match(desktopLibrary, /window_vibrancy::apply_mica\([^,]+, Some\(dark\)\)/, 'Windows 11 Mica must follow the resolved application theme');
assert.match(desktopLibrary, /apply_mica[\s\S]*is_err\(\)[\s\S]*window_vibrancy::apply_acrylic/, 'Windows 10 must fall back to a theme-aware Acrylic tint when Mica is unavailable');
assert.match(desktopLibrary, /material_transparency[\s\S]*tint_alpha/, 'Windows 10 Acrylic tint must follow material transparency');
assert.match(desktopLibrary, /sync_native_backdrop/, 'the desktop shell must expose native backdrop theme synchronization');
assert.match(desktopLibrary, /generate_handler!\[\s*sync_native_backdrop(?:,|\s*\])/, 'native backdrop synchronization must be registered as a Tauri command');
assert.match(desktopLibrary, /backend::list_backend_profiles/, 'desktop backend profile commands must be registered with Tauri');
assert.match(desktopLibrary, /backend::pair_backend_profile/, 'device pairing must be handled by the native desktop layer');
assert.doesNotMatch(backendHook, /\btoken\b/i, 'long-lived device tokens must not cross the WebView backend hook');
assert.doesNotMatch(settingsDialog, /\btoken\b/i, 'long-lived device tokens must not enter WebView settings state');
assert.ok(capability.permissions.includes('core:window:allow-close'));
assert.ok(capability.permissions.includes('core:window:allow-minimize'));
assert.ok(capability.permissions.includes('core:window:allow-toggle-maximize'));
assert.ok(capability.permissions.includes('notification:default'), 'desktop notifications must be permitted');
assert.match(cargoManifest, /tauri-plugin-notification\s*=/, 'notification plugin must be a desktop dependency');
assert.match(desktopLibrary, /tauri_plugin_notification::init\(\)/, 'notification plugin must be registered with Tauri');
assert.ok(
  windowsFrontendBuild >= 0 && windowsCredentialTest >= 0 && windowsFrontendBuild < windowsCredentialTest,
  'Windows credential tests must build frontendDist before compiling the Tauri context',
);

console.log('Desktop packaging configuration is valid.');
