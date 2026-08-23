# GPUI Desktop Client parity

This checklist defines the cutover boundary for replacing the Tauri Desktop
Client. The React application remains the Embedded WebUI. A checked item requires
both implementation and the validation named in the final column.

## Client core

| Capability | Existing source | GPUI validation |
| --- | --- | --- |
| Local and Remote Backend Profiles | `src-tauri/src/backend.rs` | lifecycle and persistence tests |
| Pairing and instance identity confirmation | `src-tauri/src/backend/lifecycle.rs` | state-machine and fake-server tests |
| Remote authenticated HTTP and WebSocket tickets | `desktop-backend.ts`, `api.ts` | protocol integration tests |
| Atomic non-secret profile persistence | `src-tauri/src/backend.rs` | failure-ordering tests |
| Platform credential storage | `src-tauri/src/backend.rs` | native adapter tests on every release OS |
| Per-backend active session and navigation state | React storage adapters | persistence tests |
| Theme, locale, terminal font, material, and close behavior | `use-preferences.ts` | round-trip and migration tests |

## Sessions and workspaces

| Capability | Existing source | GPUI validation |
| --- | --- | --- |
| Project, General, and Other navigation | `app-sidebar.tsx` | model and GPUI interaction tests |
| Sorting, manual reordering, pinning, and expansion | `app-sidebar.tsx` | persistence and pointer tests |
| Create and resume Managed Sessions | `session-dialog.tsx`, `use-sessions.ts` | fake-server integration tests |
| Search Native History | `search-dialog.tsx` | query and selection tests |
| Restart, close, details, and exited-session cleanup | `workspace-shell.tsx`, `use-sessions.ts` | lifecycle tests |
| Attention badges and deduplicated notifications | `use-sessions.ts` | clock-skew and deduplication tests |
| Session tab cycling and focus restoration | `workspace-shell.tsx` | GPUI keyboard/focus tests |

## Terminal

| Capability | Existing source | GPUI validation |
| --- | --- | --- |
| ANSI, 256 color, truecolor, attributes, alternate screen | xterm.js | byte-fixture snapshots |
| CJK, combining text, emoji, cursor, and 10k scrollback | xterm.js | model/render tests and manual IME smoke |
| Selection, clipboard, OSC 8/HTTP links | `terminal-view.tsx` | GPUI interaction tests |
| Keyboard modifiers, function keys, bracketed paste, mouse reporting | xterm.js | escape-sequence tests |
| Fit, resize, reflow, hidden-session state | `terminal-view.tsx` | resize and tab-switch tests |
| Replay replacement, reconnect, revocation, and control lease | `terminal-connection.ts` | fake-WebSocket protocol tests |
| Terminal query responses | xterm.js | `PtyWrite` protocol tests |

## Native desktop behavior

| Capability | Existing source | GPUI validation |
| --- | --- | --- |
| Single instance and focus of the existing window | Tauri single-instance plugin | native smoke test |
| Dynamic tray session menu and close-to-tray | `src-tauri/src/tray.rs` | native smoke test |
| Notifications, external links, and taskbar attention | Tauri/React adapters | native smoke test |
| Custom title bar and window controls | `window-controls.tsx` | GPUI interaction test |
| Windows Mica/Acrylic and theme restoration | `native_appearance/windows.rs` | Windows smoke test |
| Keyboard-only navigation and accessible names | Radix/React components | accessibility-tree smoke test |

## Distribution and cutover

| Capability | Existing source | GPUI validation |
| --- | --- | --- |
| Linux deb and AppImage | release workflow and Nix | install/launch smoke test |
| macOS arm64 DMG | release workflow | install/launch smoke test |
| Windows x64 NSIS | release workflow | install/launch smoke test |
| Existing Backend Profile and Windows credential migration | Tauri app config | upgrade test |
| Embedded WebUI remains independently buildable | `web/session-ui` | TypeScript build and Playwright suite |
| GPUI replaces the release entry point and Tauri is removed | release workflow | repository configuration check |

## Performance gate

Compare release builds on the same machine and backend fixture. GPUI must improve
cold start and idle RSS by at least 30%, must not regress terminal input latency or
sustained-output throughput, and must not lose terminal bytes. If this gate fails,
the release cutover does not occur.

## Cutover validation record

The feature-complete GPUI cutover was implemented on 2026-08-23. Linux x86_64
validation covered the complete Rust workspace (19 tests), strict Clippy, a clean
offline Nix release build, native window/tray and single-instance smoke checks,
and generation of both deb and AppImage packages. The retained Embedded WebUI
passed its production TypeScript/Vite build and all 29 Playwright regressions.
The daemon/TUI workspace passed formatting, check, strict Clippy, and all 121
tests.

The seven-run release benchmark measured Tauri at 198.193 ms cold start and
212196 KiB idle RSS, and GPUI at 54.770 ms and 54274 KiB. That is a 72.4% cold
start improvement and a 74.4% idle-RSS improvement. The sustained-output test
also verifies byte-for-byte preservation across a 2 MiB terminal stream.

macOS arm64, Windows x64, and Linux arm64 are mandatory release jobs. They test
or compile the same locked workspace before packaging DMG and NSIS artifacts,
but their native smoke gates cannot be executed from the Linux development host;
a release must not be published unless those jobs pass.
