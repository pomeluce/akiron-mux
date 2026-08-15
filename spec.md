# AkironMux Live Session Workbench

> Product and technical specification
>
> Version: 0.5.0
>
> Status: Implementation draft

## 1. Decision

AkironMux is the displayed product name for the live Claude Code and Codex session
workbench. It is implemented as a local background service with a browser-based
terminal interface.

The existing AkironMux TUI remains responsible for:

- API providers and model profiles
- provider switching and proxy configuration
- usage statistics
- provider-oriented history and usage views
- TUI-specific application settings

The TUI will not embed or emulate an interactive terminal. The browser workbench
is the only live-session interface.

```text
AkironMux TUI                         AkironMux WebUI
Providers / Usage / History          Live sessions + xterm.js
          |                                  |
          +------------- local API ----------+
                              |
                    akmux-sessiond daemon
                              |
                 PTY + Claude Code / Codex
```

The first implementation remains in the AkironMux repository so configuration,
history, packaging, service management, and agent launch behavior can be shared.
The user-facing executables are `akmux` and `akmux-sessiond`. Runtime data uses
the `akmux` directory and `akmux.db`; existing `akiron-mux` and `ccswitch`
directories are copied forward without deleting the legacy data.

The long-term naming scheme is:

```text
Brand           Akiron
Project         AkironMux
Repository      akiron-mux
CLI             akmux
Nix namespace   programs.akmux
```

## 2. Product Model

### 2.1 Agent

An executable AI coding tool:

```rust
enum AgentKind {
    Claude,
    Codex,
}
```

An Agent is not an API provider. Existing AkironMux providers continue to describe
API endpoints, credentials, profiles, and model catalogs.

### 2.2 Managed session

A process and PTY owned by `akmux-sessiond`:

```rust
struct SessionInfo {
    id: SessionId,
    agent: AgentKind,
    title: String,
    cwd: PathBuf,
    status: SessionStatus,
    created_at: DateTime<Utc>,
    exit_code: Option<i32>,
    error: Option<String>,
}
```

The browser never owns process or PTY handles. Closing or refreshing the browser
does not stop sessions.

### 2.3 Native history session

The existing `session_history` rows index Claude and Codex native history files.
Opening one in the WebUI creates a Managed session using the compatible native
resume command. Native history is not the source of truth for live process state.
The session service exposes these records to the WebUI and classifies them
dynamically.

### 2.4 Workspace organization

The WebUI has three directory scopes:

- **Projects** are user-created, persisted directory entries. A project owns
  history in its root and descendants. Project paths cannot overlap the General
  root or another project by ancestor/descendant relationship.
- **General** is one persisted root directory for non-project work. A new
  session may use the root directly or optionally create a named child directory
  for resource isolation.
- **Other** contains scanned native histories outside Projects and General.
  These directories are navigational records; creating new sessions there is not
  allowed. Recently opened and pinned entries are shown first.

Classification precedence is `Project > General > Other` and is recomputed after
projects or the General root changes.

### 2.5 Native titles

New-session forms do not accept a title. Until the native tool provides one, the
UI uses the project or directory name as a temporary label. Claude `custom-title`
events are authoritative. Codex `thread_name` changes are synchronized as a
best-effort signal because Codex does not expose title provenance.

### 2.6 Unified sessions

Unified means one list, workspace, API, and UI for both agents. It does not mean
Claude and Codex share native conversation context.

## 3. Process Architecture

### 3.1 Daemon

`akmux-sessiond` is the AkironMux session backend binary. It owns:

- Claude and Codex child processes
- PTY master and input handles
- bounded terminal output buffers
- live session metadata
- WebSocket clients
- HTTP API and embedded WebUI assets
- graceful process cleanup

The daemon is disabled by default. Enabling Session backend in TUI settings starts
it automatically; disabling the setting stops it. Normal use must not require
running `akmux-sessiond` manually.

### 3.2 Service lifecycle

Linux and Nix/Home Manager use a systemd user service:

```text
WantedBy=default.target
Restart=on-failure
ExecStart=akmux-sessiond
```

Later packaging may add launchd and Windows service definitions. The runtime and
HTTP service must remain platform-neutral where `portable-pty` supports the target.

Stopping the daemon closes all PTYs and child processes. Browser disconnects do
not affect process lifetime.

Normally exited Claude Code and Codex processes are removed from the managed
session list automatically after a short grace period. Failed launches remain
visible so their error can be inspected and the session can be restarted or
closed.

### 3.3 Network boundary

The service listens only on `127.0.0.1` by default. The initial stable URL is:

```text
http://127.0.0.1:17321
```

The port is configurable through `AKMUX_SESSION_PORT`; the legacy
`CCSWITCH_SESSION_PORT` remains a compatibility fallback.

Requests must reject non-local `Host` values. Browser state-changing requests and
WebSocket upgrades must reject foreign `Origin` values. CORS is not enabled.

The MVP does not support LAN or public access. Remote access requires a separate,
explicit authentication and TLS design.

## 4. Runtime Interface

The session service depends on a deep `SessionManager` interface. HTTP handlers do
not access PTY or child-process objects.

```rust
impl SessionManager {
    fn list(&self) -> Vec<SessionInfo>;
    fn create(&self, request: CreateSession) -> Result<SessionInfo>;
    fn get(&self, id: &SessionId) -> Option<SessionHandle>;
    fn restart(&self, id: &SessionId) -> Result<()>;
    fn close(&self, id: &SessionId) -> Result<()>;
    fn shutdown(&self);
}

impl SessionHandle {
    fn write(&self, bytes: Vec<u8>) -> Result<()>;
    fn resize(&self, rows: u16, cols: u16) -> Result<()>;
    fn subscribe(&self) -> broadcast::Receiver<SessionStreamEvent>;
    fn scrollback(&self) -> Vec<u8>;
}
```

Each session has independent command routing and output buffering. One blocked or
failed session must not block manager commands for other sessions.

Terminal output is treated as bytes. ANSI parsing and rendering belong to
xterm.js, not the Rust service.

## 5. Agent Drivers

Agent-specific executable behavior is isolated behind drivers:

```text
Claude new       claude
Claude picker    claude --resume
Claude resume    claude --resume <native-id>
Codex new        codex
Codex picker     codex resume
Codex resume     codex resume <native-id>
```

Drivers own executable names, arguments, resume syntax, and future environment
overrides. Secrets are inherited from the user's configured environment and are
never returned through the session API.

## 6. HTTP and WebSocket API

### 6.1 HTTP endpoints

```text
GET    /api/health
GET    /api/directories?path=<encoded-directory>
GET    /api/sessions
POST   /api/sessions
POST   /api/sessions/:id/restart
DELETE /api/sessions/:id
GET    /api/sessions/:id/terminal    WebSocket upgrade
GET    /api/workspaces
POST   /api/projects
PATCH  /api/projects/:id
DELETE /api/projects/:id
GET    /api/history
POST   /api/history/refresh
GET    /api/settings
PATCH  /api/settings
```

Create request:

```json
{
  "agent": "codex",
  "title": "",
  "cwd": "/home/user/project",
  "rows": 30,
  "cols": 100,
  "resume": false,
  "resume_id": null
}
```

Set `resume` to `true` with no `resume_id` to launch the agent's native history
picker. A non-null `resume_id` takes precedence and resumes that exact native
session.

The directory endpoint canonicalizes the requested path and returns its parent,
the user's home directory, and sorted child directories. It never returns files.
Dot-prefixed directories are hidden by default and can be included with the
`show_hidden=true` query parameter.
This endpoint powers the in-app directory browser and avoids relying on a native
browser file picker, which is unsuitable when the service runs in WSL or another
host environment.

### 6.2 WebSocket protocol

Server to browser:

- binary frames: raw PTY output
- text frames: JSON status events

Browser to server:

- binary frames: UTF-8 terminal input
- text frames: resize or control JSON

```json
{ "type": "resize", "rows": 30, "cols": 100 }
```

On connection, the server first sends the bounded scrollback buffer and current
status. A slow browser may lose intermediate broadcast frames and must reconnect to
receive the latest scrollback state.

## 7. WebUI

The browser interface is a workbench, not a landing page.

```text
+-----------------------------------------------------------------------+
| AkironMux                                      Language  Theme         |
+--------------------+-----------------------------+--------------------+
| Sessions   History +| implementation   Running   | Session details    |
| All  Claude  Codex  +-----------------------------+                    |
| implementation     >|                             | Agent              |
| review               |      xterm.js terminal     | Status             |
| tests                |                             | Working directory  |
|                      |                             |                    |
+--------------------+-----------------------------+--------------------+
| AkironMux version                working directory        session count|
+-----------------------------------------------------------------------+
```

Required MVP behavior:

- list all Claude and Codex sessions
- filter and search unified Claude/Codex native history
- create a session with an agent and a directory scope; General optionally creates
  a named child directory
- open a concrete native history record with its exact resume ID
- choose the working directory through a custom in-app directory browser
- show or hide dot-prefixed directories in the directory browser
- organize history as Project, General, and Other directory groups
- label Projects as Workspaces in the client while retaining Project as the
  backend domain term
- show Workspace and General section `...`/`+` icon actions with tooltips
- let users edit a Workspace name, path, and icon, and override General/Other
  directory icons; icon overrides are client-owned appearance preferences
- switch active terminals without stopping inactive sessions
- preserve browser terminal state while switching
- resize the active PTY using xterm fit measurements
- restart failed sessions
- confirm before closing a running session
- reconnect after page refresh
- automatically remove normally exited sessions
- display reliable Starting, Running, Exited, and Error states
- support Material Design 3 light and dark application themes
- support English and Simplified Chinese interface text

The terminal remains the dominant surface and fills the available central
workspace. The shell follows a Codex Desktop-like workbench structure: a compact
application title bar, a translucent acrylic navigation sidebar with Projects,
General, Other, and Settings, and a raised main work surface. Claude/Codex output
and session details share that work surface; details open as an internal drawer
instead of a fixed global inspector.
The application uses restrained Material Design 3 tokens, stable pane dimensions,
and accessible control sizes. Claude Code and Codex terminal output keeps its
native ANSI and truecolor palette independently of the light or dark application
shell. The WebUI does not duplicate provider editing or usage charts. Its settings
modal contains client preferences plus backend-owned General root and project
organization settings.

Theme and language preferences are stored locally in the browser. The initial
theme follows the operating-system preference, and the initial language follows
the browser language when no saved choice exists.

## 8. Output and Resource Limits

Each session keeps a bounded raw output buffer. The MVP limit is 1 MiB per session.
When the limit is exceeded, the oldest bytes are discarded.

WebSocket broadcasts use bounded channels. Slow clients must not create unbounded
memory growth or slow the PTY reader.

The service limits title and path request sizes and rejects nonexistent working
directories before process creation.

## 9. Persistence

Live PTY metadata remains in memory, but workspace organization and settings are
persisted in SQLite immediately. The service imports native history on startup,
when directory roots change, and on explicit refresh. Managed sessions are not
claimed to survive a daemon restart; native history remains resumable.

## 10. Packaging

The package contains:

```text
bin/akmux
bin/akmux-sessiond
lib/systemd/user/akmux-sessiond.service
```

The production WebUI build is embedded in `akmux-sessiond`. Runtime use does not
require Node.js, npm, a source checkout, or network access.

Home Manager installs the AkironMux session service unit when configured. The
persisted TUI setting remains authoritative: unless Session backend is enabled,
the service exits without binding port 17321.

The standalone desktop package uses Tauri 2 and remains separate from
`akmux-sessiond`. The first packaged target is an unsigned Windows x64 NSIS test
installer built from WSL with `cargo-xwin`; native code signing remains a release
pipeline concern. The desktop application defaults to `http://127.0.0.1:17321`
and can connect to another explicitly configured loopback backend address.
Windows packages use a per-machine NSIS installation under Program Files and
therefore request administrator elevation. The installer, executable, taskbar,
and in-application brand mark are generated from the same transparent
`web/session-ui/public/akiron.svg` source.

The standalone desktop package changes the top-right primary action to a
settings button while keeping New session and History in the session panel. Its
settings surface includes:

- default session working directory
- backend base URL and connection test
- optional acrylic or backdrop blur for non-terminal application surfaces
- theme and language preferences

On Windows, acrylic is applied by the native Tauri window behind one shared
translucent application background. The transparency setting controls surface opacity;
the terminal surface stays opaque so ANSI colors and text remain legible. The
desktop window uses an application-rendered title bar with connection status and
minimize, maximize/restore, and close controls. The sidebar can be resized from
its right edge, persists its width locally, and is capped at one third of the
window width. The backend accepts CORS requests only from loopback browser
origins and the fixed Tauri application origins. External web origins remain
rejected.

## 11. Delivery Plan

### Phase 1: daemon and one terminal

- agent drivers
- PTY session runtime
- bounded output buffer
- HTTP health and session APIs
- one WebSocket terminal
- xterm.js WebUI
- loopback and Origin/Host validation

### Phase 2: multi-session workbench

- concurrent sessions
- filtering and switching
- restart and confirmed close
- reconnect and scrollback replay
- responsive desktop/mobile layout
- Material Design 3 light and dark themes
- English and Simplified Chinese localization
- custom directory browser
- automatic cleanup of normally exited sessions

### Phase 3: product integration

- systemd and Home Manager startup
- packaged embedded assets
- TUI History action opens/resumes in WebUI
- diagnostics and service status

### Phase 4: workspace integration

- project, General, and Other workspace migrations
- native history browsing and exact resume
- settings persistence and sorting/pinning
- native title synchronization
- bounded transcript export

### Phase 5: structured agents

- Codex App Server transport
- Claude structured transport
- normalized tool, approval, usage, and task events

### Phase 6: standalone desktop frontend

- Tauri-based separately packaged AkironMux desktop application
- Windows x64 NSIS test package built independently from `akmux-sessiond`
- configurable backend address and connection diagnostics
- per-machine NSIS installation and shared transparent application icons
- custom desktop title bar and native Windows acrylic effects
- resizable sidebar capped at one third of the application width
- desktop settings surface and adjustable non-terminal transparency
- existing embedded WebUI retained for direct browser access

## 12. MVP Acceptance Criteria

1. `akmux-sessiond` starts without launching the TUI when the backend setting is enabled.
2. The service binds only to the loopback address.
3. The WebUI loads without external network dependencies.
4. Claude Code and Codex can each start inside xterm.js.
5. Terminal input, ANSI output, cursor behavior, paste, and resize work.
6. At least three sessions can run concurrently.
7. Refreshing or closing the browser does not stop sessions.
8. Switching sessions does not mix terminal output.
9. Normally exited sessions disappear automatically; failed sessions remain
   manageable and can be restarted or closed.
10. Stopping the daemon cleans up managed child processes.
11. Foreign Host and Origin values are rejected.
12. The application supports light/dark themes and English/Simplified Chinese.
13. Working directories can be selected using the in-app directory browser.
14. Project, General, and Other history groups classify dynamically.
15. Existing TUI and CLI behavior remains unchanged.

## 13. Validation

Required automated checks:

```text
cargo fmt --all
cargo check --offline
nix develop -c cargo clippy --all-targets --offline -- -D warnings
cargo test --offline
pnpm build
git diff --check
```

Runtime validation uses deterministic local shell fixtures rather than requiring
Claude or Codex credentials in CI. Browser validation covers desktop and mobile
layouts, terminal connection, create, switch, resize, restart, and close flows.
