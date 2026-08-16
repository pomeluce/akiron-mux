import assert from 'node:assert/strict';
import fs from 'node:fs';

const config = JSON.parse(fs.readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const capability = JSON.parse(fs.readFileSync(new URL('../src-tauri/capabilities/main-window.json', import.meta.url), 'utf8'));
const cargoManifest = fs.readFileSync(new URL('../src-tauri/Cargo.toml', import.meta.url), 'utf8');
const desktopLibrary = fs.readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8');
const window = config.app.windows.find(candidate => candidate.label === 'main');
const nsis = config.bundle.windows.nsis;

assert.equal(nsis.installMode, 'perMachine', 'Windows installer must target Program Files');
assert.equal(nsis.installerIcon, 'icons/icon.ico', 'NSIS installer must use the application icon');
assert.equal(window.transparent, true, 'native backdrop materials require a transparent Tauri window');
assert.equal(window.decorations, false, 'desktop builds must use the custom title bar');
assert.match(cargoManifest, /\[target\.'cfg\(target_os = "windows"\)'\.dependencies\][\s\S]*window-vibrancy\s*=/, 'Windows backdrop materials must use the native vibrancy API');
assert.match(desktopLibrary, /window_vibrancy::apply_mica\(&window, None\)/, 'Windows 11 must use the wallpaper-based Mica material');
assert.match(desktopLibrary, /apply_mica[\s\S]*is_err\(\)[\s\S]*window_vibrancy::apply_acrylic/, 'Windows 10 must fall back to Acrylic when Mica is unavailable');
assert.ok(capability.permissions.includes('core:window:allow-close'));
assert.ok(capability.permissions.includes('core:window:allow-minimize'));
assert.ok(capability.permissions.includes('core:window:allow-toggle-maximize'));
assert.ok(capability.permissions.includes('notification:default'), 'desktop notifications must be permitted');
assert.match(cargoManifest, /tauri-plugin-notification\s*=/, 'notification plugin must be a desktop dependency');
assert.match(desktopLibrary, /tauri_plugin_notification::init\(\)/, 'notification plugin must be registered with Tauri');

console.log('Desktop packaging configuration is valid.');
