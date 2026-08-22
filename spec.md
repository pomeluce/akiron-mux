# AkironMux Live Session Workbench

> Product and technical specification
>
> Version: 0.6.0
>
> Status: Implementation draft

## 1. Decision

AkironMux is the displayed product name for the live Claude Code and Codex session
workbench. It consists of a host-side session daemon, the existing TUI, an
embedded local WebUI, and separately installed desktop and mobile GUI clients.

The existing AkironMux TUI remains responsible for:

- API providers and model profiles
- provider switching and proxy configuration
- usage statistics
- provider-oriented history and usage views
- TUI-specific application settings

The TUI will not embed or emulate an interactive terminal. Installed GUI clients
are the primary live-session interface. The daemon's embedded WebUI remains
available only through the local listener for loopback use.

```text
AkironMux TUI                  Desktop / Android GUI
Providers / Usage / History    Live sessions + xterm.js
          |                       | local or HTTPS/WSS
          +-----------------------+
                         |
               akmux-sessiond daemon
                 |               |
        local listener      remote listener
          127.0.0.1        authenticated API
                 \               /
                  PTY + Claude Code / Codex
```

The implementation remains in the AkironMux repository so configuration,
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

Canonical product terms are defined in [CONTEXT.md](CONTEXT.md). This section
specifies the behavior and data contracts that apply to those concepts.

### 2.1 Agent

The supported Agent kinds are:

```rust
enum AgentKind {
    Claude,
    Codex,
}
```

Selecting an Agent does not select or replace an API Provider. Existing AkironMux
providers continue to describe API endpoints, credentials, Model Profiles, and
model catalogs.

### 2.2 Managed session

The Managed Session data contract is:

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

Clients never own process or PTY handles. Closing or refreshing a client does not
stop Managed Sessions.

### 2.3 Native history session

The existing `session_history` rows index Claude and Codex native history files.
Opening one in the WebUI creates a Managed Session using the compatible native
resume command. Native History Sessions are not the source of truth for live
process state.
Native History Ingestion uses one typed Agent interface for discovery, revision,
cleanup, persistence, and progress. Claude and Codex file formats are internal
adapters. Session and usage revisions are independent. Each changed file and its
sync index commit atomically; an unreadable or malformed file retains its last
valid indexed state while database failures stop the ingestion run.
The session service exposes these records to the WebUI and classifies them
dynamically. Codex internal child threads identified by `parent_thread_id` are
hidden from native history and contribute their message and usage totals to the
parent thread. Explicit `/fork` sessions remain independent history records.

Opening Native History is idempotent by Native Session Key. A matching Managed
Session in Starting, Running, or Error state is selected instead of starting a
second Agent process; only an exited and removed session may be resumed again.
The backend enforces this invariant across clients and concurrent requests. The
client keeps the active matching instance when possible and otherwise selects
the newest match. It does not automatically close duplicate instances left by an
older client or backend.

### 2.4 Workspace organization

The WebUI organizes directories into the Project, General, and Other scopes
defined in [CONTEXT.md](CONTEXT.md):

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

### 2.6 Backend connection

An installed GUI persists a list of backend connection profiles:

```rust
enum BackendKind {
    Local,
    Remote,
}

struct BackendProfile {
    id: BackendProfileId,
    name: String,
    kind: BackendKind,
    base_url: Url,
    credential_ref: Option<CredentialRef>,
    expected_instance_id: Option<BackendInstanceId>,
}
```

A Local profile accepts only `localhost`, `127.0.0.1`, or `[::1]` and does not
require a device credential. Desktop clients contain one built-in Local profile
that cannot be deleted, although its loopback port may be edited. Android does
not create a Local profile because its loopback interface is the phone itself.

A Remote profile requires an HTTPS base URL and a device credential. Profile
names are unique within one client. A client may store multiple profiles but
connects to only one backend at a time. Switching profiles disconnects GUI HTTP
and WebSocket activity from the previous backend without stopping its managed
sessions. An unreachable selected backend remains selected and shows a recoverable
error; the client never silently falls back to Local.

Non-secret profile metadata, ordering, and the active profile are client-owned.
Installed clients persist them through a Tauri-native configuration store. Device
credentials are stored separately in the operating-system credential store. A
credential-store failure permits an in-memory connection for the current process
only and must never fall back to `localStorage` or a plaintext configuration file.

The native Backend Profile Lifecycle is the authority for pairing, Backend
Instance ID confirmation, capability validation, activation, refresh, and
revocation ordering. WebView callers submit typed intents and receive typed
outcomes for identity confirmation, authentication requirements, offline
selection, and revocation warnings; expected lifecycle states are not encoded in
error strings. Identity confirmation uses an opaque, single-use native challenge
that expires after two minutes and is bound to the pending operation.

Profile metadata is replaced atomically. Pairing stores the Device Credential
before committing metadata and removes the new credential if that commit fails.
Deletion destroys the local credential before committing metadata removal; a
metadata failure therefore leaves a profile that requires re-authentication,
never a silently usable credential. Server-side revocation is best-effort and a
failure is returned as a typed warning after local deletion completes.

On upgrade, an existing loopback `akironmux-backend-address` value is imported
into the built-in Local profile. A non-loopback legacy value requires explicit
re-creation and authentication. Backups may migrate non-secret profiles, but a
restored Remote profile is marked as requiring authentication.

Each backend profile remembers its last active session and navigation expansion
state. Theme, language, terminal font size, material opacity, and sidebar width
remain global client preferences. Server-owned workspace sorting remains on the
selected backend.

### 2.7 Session attention

Session attention is independent of Managed Session process state. Interaction
Attention is emitted whenever the primary Agent or one of its Child Agent Runs
requires user input or permission. Completion Attention is emitted only after a
Primary Agent Turn has finished and no Child Agent Run remains active. Child
completion and Agent process exit do not produce Completion Attention.

Agent adapters classify completion before broadcasting attention to clients, so
in-application markers, taskbar attention, and system notifications share the
same behavior. Codex completion uses the event `thread-id` and the canonical first
`session_meta`: a non-empty `parent_thread_id` identifies an internal child and is
suppressed, while an explicit `/fork` remains independent. Missing or unreadable
metadata is retried briefly and then fails closed with a diagnostic entry. Claude
registers `Stop`, never `SubagentStop`, and suppresses a `Stop` while its hook
payload reports active agent-like background work; an ordinary background shell
task alone does not suppress completion.

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

The daemon owns two independent listeners. Local and Remote are explicit security
modes, not classifications inferred from a request's source address.

```text
Local   http://127.0.0.1:17321   loopback only, embedded WebUI, no device token
Remote  http://<bind>:17322      API only, every operation authenticated
```

The Local port remains configurable through `AKMUX_SESSION_PORT`; the legacy
`CCSWITCH_SESSION_PORT` remains a compatibility fallback. The Local listener
accepts only loopback Host values and trusted loopback or packaged-application
origins. It is the only listener that serves embedded WebUI assets.

The Remote listener has an independently configurable bind address and defaults
to disabled. It may bind to loopback for Caddy or Tailscale Serve, or to a specific
private, LAN, or Tailnet address. Binding `0.0.0.0` is an advanced operation that
requires an explicit high-risk confirmation. A Remote listener must not start
without at least one valid device credential. Public IP bindings are rejected by
default. The documented deployment boundary requires a firewall, Tailnet, or TLS
reverse proxy and must never expose the plaintext listener directly to the public
Internet.

The daemon does not terminate TLS in the first Remote release and does not read
certificate private keys. Installed clients connect to a configured public URL
such as `https://host.example.com`; that URL is separate from the bind address and
is required for Host validation, diagnostics, and pairing payloads. It must use
HTTPS and contain no user information, path, query, or fragment. Caddy and
Tailscale Serve examples are documented, but AkironMux does not automatically
modify proxies, DNS, firewalls, or Tailnet configuration.

Remote request validation does not trust `X-Forwarded-For`, `X-Forwarded-Host`,
or related proxy headers. Host is matched against the configured public URL and
Origin remains a browser-isolation check rather than an identity mechanism.
The authenticated device identity is the authorization boundary.

### 3.4 Service configuration and lifecycle

The TUI's Session backend setting is the service master switch. Disabling it
stops both listeners. Enabling it starts Local; a separate Remote setting controls
the Remote listener. Remote bind address, port, public URL, enabled state, device
credential records, and audit metadata are stored in `akmux.db`. Token plaintext
is never stored there.

Changing Remote listener settings must not terminate managed Claude or Codex
processes. The preferred implementation dynamically adds, replaces, or removes
the Remote listener inside the running daemon. Until that is available, a change
that would restart the daemon is rejected while managed sessions are running and
the user is instructed to close them explicitly. Silent session termination is
not allowed.

CLI provides the complete configuration, device, pairing, diagnostics, and audit
surface. TUI settings provides the master and Remote switches, listener status,
QR pairing, and common device-revocation actions. Complex proxy diagnostics and
bind configuration remain in CLI.

### 3.5 Backend identity and protocol compatibility

Each daemon persists a random backend instance ID and reports an explicit API
protocol version and capability set after authentication. A client pins the
instance ID when saving a profile. If the same URL later returns a different ID,
the client requires explicit confirmation before replacing the pin.

An incompatible API protocol major version rejects the connection. Minor-version
differences use advertised capabilities to disable unsupported features. Remote
clients never downgrade security features such as device authentication,
WebSocket tickets, or control leases. A loopback Local client may enter a clearly
labelled compatibility mode for an older backend.

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

### 6.1 Authentication model

Remote device credentials are host-control credentials. They allow directory
browsing and creation, Claude/Codex process creation and management, terminal
output access, and arbitrary PTY input under the daemon user's permissions. The
product must describe this scope directly and must not offer a misleading
read-only device Token.

Each installed client device has an independent credential record containing a
device name, public Token ID, creation time, last-used time, and active or revoked
state. Credentials are long-lived until explicitly revoked. The printable format
is versioned and contains a public lookup ID plus at least 256 bits of random
secret material:

```text
akmux_1_<token-id>_<secret>
```

The daemon stores only an HMAC-SHA-256 digest indexed by Token ID. The HMAC key is
a random server pepper held in a separate, permission-restricted file.
Verification uses a constant-time comparison. The
database and pepper must be backed up together; losing the pepper invalidates all
device credentials and requires re-pairing. Neither credentials nor terminal
content may appear in logs, diagnostics, exports, or audit records.

Every Remote `/api/*` request requires `Authorization: Bearer <device-token>`.
Missing or invalid authentication returns `401`; an authenticated identity without
permission returns `403`. Authentication covers read and write routes, including
health details, directory browsing, settings, history, and terminal access. CORS
preflight does not require a Bearer Token but validates the configured Origin,
method, and headers exactly and permits `Authorization` explicitly.

The only anonymous Remote routes are:

```text
GET  /healthz     reachability only; no version, path, or host details
POST /api/pair    consumes an unexpired pairing request
```

Authentication and pairing failures use an in-memory source and Token/pairing-ID
rate limiter with exponential backoff. Restarting the daemon clears the limiter;
malicious traffic must not permanently lock out valid devices. HTTP request body
sizes, concurrent WebSockets, and WebSocket idle time are bounded.

### 6.2 Pairing and device lifecycle

The normal mobile pairing flow is:

```text
CLI/TUI creates a 60-second pending pairing request
        -> displays a local QR code
phone scans akmux://pair payload
        -> submits device name and pairing code
CLI/TUI displays the request source and asks for confirmation
        -> user accepts
phone receives a device Token directly into secure storage
        -> pairing request becomes permanently invalid
```

The QR payload includes only the HTTPS public URL, backend display name, and
single-use pairing code. It never contains a long-lived device Token and is
generated locally without uploading data. Unknown deep-link parameters,
non-HTTPS URLs, reused codes, and expired codes are rejected. An unconfirmed or
unclaimed request expires after 60 seconds without creating an active credential.

For headless servers and automation, an explicitly requested interactive CLI
command may create a device and print a complete Token exactly once:

```text
akmux backend device create --name <name> --show-token
```

The command refuses non-interactive output by default. A revoked device loses
authorization immediately: the daemon closes its active HTTP/WebSocket control
connections and terminal subscriptions without stopping managed agent processes.
The audit log records device creation, pairing, last use, revocation, and failed
authentication metadata for 30 days. It never records Token material, terminal
input, terminal output, or commands.

Deleting a Remote profile from a GUI removes the local secure credential and, by
default, requests server-side device revocation. If the backend is unreachable,
the local profile can still be removed but the client warns that server-side
revocation is unconfirmed. Deleting a profile never closes remote managed sessions.

### 6.3 HTTP endpoints

```text
GET    /api/health
POST   /api/auth/ws-ticket
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

Authenticated health and capability discovery returns the backend instance ID,
application version, API protocol version, capability set, and host metadata
needed to verify a connection. Adding or editing a Remote profile must pass this
authenticated connection test before it can be saved. Local profiles may be
saved while their service is offline. Backends that support explicit terminal
snapshot framing advertise the `terminal-replay-v1` capability.

### 6.4 WebSocket authentication and protocol

A Remote client first exchanges its Bearer Token over HTTPS for a WebSocket
ticket. A ticket is valid for 30 seconds, can be used exactly once, and is bound
to one device and one managed session. Long-lived credentials are never placed in
a URL or sent as a WebSocket frame. Reconnection obtains a new ticket.

Server to browser:

- binary frames: raw PTY output
- text frames: typed JSON events for `replay`, `status`, `lease`,
  `lease-recovery`, `attention`, `authorization-revoked`, and `protocol-error`

Browser to server:

- binary frames: UTF-8 terminal input
- text frames: resize or control JSON

```json
{ "type": "resize", "rows": 30, "cols": 100 }
```

On connection, the server sends the current lease first so an eligible writer can
synchronize the PTY size before output is rendered. It then sends a `replay`
event with `replace: false`, the bounded scrollback as one binary frame, and the
current status. The binary frame is present even when the snapshot is empty, so
the replay marker always applies to exactly one frame.

If a browser falls behind the bounded live-output channel, the server sends a
fresh `replay` event with `replace: true` followed by the latest scrollback
snapshot. The client replaces its terminal buffer without reconnecting. Clients
accept the legacy `reset` event as a replacement marker during minor-version
compatibility. Unknown event or control types are ignored and logged; malformed
known controls receive `protocol-error` without terminating the managed session.

### 6.5 Terminal control lease

Multiple clients may observe one managed session, but only one concrete terminal
WebSocket connection holds its control lease and may send PTY input. The first
eligible connection obtains control; other connections remain read-only and may
scroll, copy, inspect details, or request control. Lease state includes a monotonic
version and the controlling device name and is broadcast to all viewers.

Taking control atomically transfers the lease. The previous connection remains
open, immediately becomes read-only, and displays which device took control. Two
concurrent takeovers are ordered by the server: the first request succeeds and a
later requester receives the latest lease state. The default policy permits
immediate takeover with notification; a backend setting may require confirmation
from the current controller.

The lease belongs to a WebSocket connection, not merely a device, so two windows
on one computer cannot both write. Local clients participate using a temporary
local device identity without a persistent Token. Disconnecting grants a 30-second
recovery window. A reconnect using the same connection recovery credential may
restore control during that window; after expiry it reconnects read-only and may
request takeover. Network changes follow the same rule.

## 7. GUI and Embedded WebUI

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

### 7.1 Backend selection and management

Installed desktop and mobile clients expose the active backend in the title or
top application bar. The selector shows profile name, Local or Remote type, and
connection state and provides fast switching. Settings owns profile creation,
editing, ordering, connection testing, re-authentication, and deletion.

Only the selected backend has active GUI polling and terminal WebSockets. Leaving
a backend does not stop its sessions, and the first release does not generate
client notifications for non-active backends. Switching is blocked while an IME
composition is active and instructs the user to commit or cancel the composition;
normal terminal characters have already been streamed and are not replayed.

The Unified Sessions client publishes one snapshot tagged with the selected
Backend Profile and a monotonically increasing generation. A backend switch
immediately hides the previous live session list and restores only the target
profile's persisted active-session hint; the target backend must provide a fresh
Managed Session list. Results from create, resume, close, and polling requests
belonging to an older generation are ignored.

Tabs, keyboard cycling, tray actions, creation, and native-history resume use one
user-selection intent that clears Managed and Native attention, persists the
selection for that Backend Profile, and focuses the terminal. Restoring a saved
selection and reconciling a polling response do not request focus. Removing the
active session through close or process exit selects and focuses its adjacent
session. Attention freshness and duplicate suppression are Unified Sessions
policy; platform notification and tray APIs remain client adapters.

Backend reconnect uses exponential backoff capped at 30 seconds. Reconnection
does not reload the React application or discard xterm scrollback, UI state, or
an unfinished IME composition. After reconnecting, the client retrieves bounded
server scrollback, session state, capabilities, and the current control lease.

### 7.2 System material and opacity

The appearance setting is named Background material and Material transparency,
not Acrylic. Its meaning is consistent across native backdrops:

```text
0%     fully opaque AkironMux background color
100%   un-tinted native system material
```

On Windows 11 the native material is Mica. Windows 10 falls back to Acrylic.
The native material follows AkironMux's resolved light or dark theme rather than
the Windows system theme, including when the user overrides the application theme.
Switching the application theme updates both WebUI color tokens and
`MicaLight`/`MicaDark` or the corresponding Acrylic tint.

The desktop native-appearance layer owns resolved appearance state, platform
application, and restoration after window events. Windows-specific DWM messages
and backdrop APIs remain inside its Windows adapter; other platforms use the
same application seam without compiling Windows implementation details.

The WebView overlay interpolates from an application tint to transparent. The
dark tint uses a deeper neutral black so increasing transparency does not produce
a pale or washed-out dark shell. The light tint uses a neutral light gray. The
terminal stays opaque so ANSI and truecolor output remains stable. Floating
dialogs retain a lightly translucent, more opaque surface for legibility.

### 7.3 Installed-client credential handling

Remote device credentials never enter WebView `localStorage`. Desktop clients use
the operating-system credential manager; Android uses hardware-backed Keystore
where available, and iOS later uses Keychain. Frontend state stores only a
credential reference. Secrets are masked, excluded from logs and error objects,
and cleared from application memory when practical after use.

Android offers an optional per-backend requirement for biometric or device-lock
confirmation before connecting. It is disabled by default. A per-backend screen
capture protection option is also disabled by default and applies to terminal and
pairing surfaces when enabled.

## 8. Android Client

### 8.1 Release scope

The first mobile target is Android 10 and newer. It is distributed initially as
signed GitHub Release APKs: a recommended `arm64-v8a` package and a universal APK
containing `arm64-v8a`, `armeabi-v7a`, and `x86_64`. GitHub Actions receives a
stable release keystore through repository secrets; the keystore has an offline
backup because losing it prevents seamless upgrades. Google Play distribution
and iOS are later phases.

The Android application is a Tauri mobile client of a Remote backend. It does not
contain or launch `akmux-sessiond`. First launch offers QR scan, manual HTTPS URL
and Token entry, and pairing-text import from the clipboard. Camera permission is
requested only after the user chooses scan. Successful manual Token import stores
the credential in Keystore and attempts to clear the clipboard, while explaining
that clipboard clearing is not reliable on every Android vendor.

### 8.2 Mobile workbench

Mobile uses a dedicated terminal-first layout rather than a compressed desktop
window. It has:

- a top bar for backend and session selection
- a side drawer for Workspaces, General, Other, and session history
- a full-width, full-height terminal using safe-area and dynamic viewport insets
- a keyboard toolbar with Esc, Tab, Ctrl, arrow keys, paste, and hide keyboard
- portrait and landscape layouts with a collapsible toolbar in landscape

`Ctrl` is a one-shot modifier for the next key and locks when double-tapped. The
terminal validates software-keyboard composition, Chinese IME input, long-press
selection, copy and paste, `visualViewport` resizing, and reconnection after
Wi-Fi/cellular transitions. Touch targets meet mobile accessibility sizing.
Pointer-based desktop sorting must not interfere with drawer scrolling.

The first Android release supports directory browsing, Workspace/history viewing,
new session creation, and exact native-session resume. Workspace editing, icon
editing, and manual drag sorting are hidden until a later mobile release.

Android back navigation and application exit disconnect the client without
closing any managed session. Closing Claude or Codex requires the explicit Close
session action.

### 8.3 Mobile notification boundary

While the application is active, events for an unfocused session use native local
notifications and distinct interaction-required and session-completed signals.
When the application is backgrounded, WebSocket delivery and local notification
creation are best-effort only. The first Android release does not promise that an
authorization request or completion alert will arrive while suspended or closed.

Reliable background delivery requires a later FCM/APNs service, device push-token
registration, server-side event delivery, and notification deep links. iOS must
not rely on a persistent background WebSocket. This limitation is stated in
settings and release documentation.

## 9. Output and Resource Limits

Each session keeps a bounded raw output buffer. The MVP limit is 1 MiB per session.
When the limit is exceeded, the oldest bytes are discarded.

WebSocket broadcasts use bounded channels. Slow clients must not create unbounded
memory growth or slow the PTY reader.

The service limits title and path request sizes and rejects nonexistent working
directories before process creation.

## 10. Persistence

Live PTY metadata remains in memory, but workspace organization and settings are
persisted in SQLite immediately. The service imports native history on startup,
when directory roots change, and on explicit refresh. Managed sessions are not
claimed to survive a daemon restart; native history remains resumable.

## 11. Packaging

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
`akmux-sessiond`. Windows x64 is distributed as an NSIS installer. The desktop
application contains the built-in `http://127.0.0.1:17321` Local profile and can
store multiple authenticated Remote profiles.
Windows packages use a per-machine NSIS installation under Program Files and
therefore request administrator elevation. The installer, executable, taskbar,
and in-application brand mark are generated from the same transparent
`web/session-ui/public/akiron.svg` source.

The standalone desktop package changes the top-right primary action to a
settings button while keeping New session and History in the session panel. Its
settings surface includes:

- default session working directory
- backend profile management and connection diagnostics
- background material and material transparency
- theme and language preferences

On Windows 11, Mica is applied by the native Tauri window behind one shared
translucent application background; Windows 10 uses Acrylic as a compatibility
fallback. The material follows the application's resolved theme. The
desktop window uses an application-rendered title bar with connection status and
minimize, maximize/restore, and close controls. The sidebar can be resized from
its right edge, persists its width locally, and is capped at one third of the
window width.

The Android package is a separately signed Tauri application and contains no
daemon binary. Mobile and desktop assets derive from the same offline application
icon source. Platform-specific Tauri configuration separates desktop window
effects and title-bar permissions from Android safe-area, notification, camera,
deep-link, credential-store, and screen-capture capabilities.

## 12. Delivery Plan

Phases 1 through 6 describe the implemented Local workbench baseline. Phases 7
through 11 define the approved extension plan and are delivered independently so
each security boundary can be tested and rolled back.

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
- responsive narrow desktop/browser layout
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
- custom desktop title bar and native Windows backdrop effects
- resizable sidebar capped at one third of the application width
- desktop settings surface and adjustable non-terminal transparency
- existing embedded WebUI retained for direct browser access

### Phase 7: system material correction

- synchronize Mica or Acrylic light/dark mode with the resolved AkironMux theme
- redefine the opacity slider as opaque application tint to pure native material
- deepen the dark overlay tint and keep terminal and dialog contrast stable
- rename Acrylic settings to platform-neutral Background material terminology

### Phase 8: authenticated Remote backend

- independent Local and Remote listeners and dynamic listener lifecycle
- Remote public URL, Host validation, API protocol, capabilities, and instance ID
- per-device versioned Tokens with keyed digests and external pepper
- authenticated HTTP, anonymous minimal health, rate limiting, and audit records
- 60-second QR pairing with CLI/TUI confirmation and headless Token creation
- 30-second one-use WebSocket tickets and immediate device revocation
- terminal control leases, takeover state, read-only observers, and recovery grace
- Caddy examples, Tailscale Serve guidance, and connection diagnostics

### Phase 9: multi-backend installed clients

- Local and Remote backend profile model with one active backend
- Tauri-native non-secret profile persistence and legacy loopback migration
- Windows credential storage with temporary in-memory fallback only
- active-backend selector, connection testing, identity pinning, and re-authentication
- profile-scoped active-session/navigation state and reconnect without UI reload
- profile deletion with default server-side device revocation

### Phase 10: Android client

- Tauri Android project, platform configuration, capabilities, and Keystore storage
- QR/deep-link, manual, and clipboard pairing flows
- terminal-first portrait and landscape UI with safe-area and keyboard handling
- mobile terminal shortcut toolbar, IME, selection, paste, and network recovery
- Android 10+ smoke and UI tests
- signed arm64 and universal APK GitHub Release artifacts

### Phase 11: iOS and reliable push

- iOS-specific layout, Keychain, signing, packaging, and App Store preparation
- FCM/APNs device registration and server-side background event delivery
- notification deep links and documented delivery semantics
- optional multi-backend background monitoring after the push security model is reviewed

## 13. Acceptance Criteria

### 13.1 Local workbench baseline

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
16. Clicking a Native History Session selects its existing non-exited Managed
    Session without starting a duplicate Agent process, including concurrent and
    multi-client resume requests.
17. Child Agent Run completion never produces Completion Attention, while child
    permission or input requests still produce Interaction Attention.

### 13.2 System material

1. Material transparency spans a visibly opaque application tint at `0%` and
   pure native material at `100%` without changing the terminal surface.
2. Windows 11 Mica and Windows 10 Acrylic follow the AkironMux resolved theme even
   when it differs from the Windows system theme.
3. Dark mode remains deep and neutral at high transparency instead of becoming
   pale or washed out.
4. In dark mode, taskbar previews and redraws triggered by Windows window events
   do not expose a light fallback background.
5. Selecting a Managed Session with its tab or with `Ctrl+Tab` or
   `Ctrl+Shift+Tab` focuses the selected terminal for immediate input.

### 13.3 Remote backend security

1. Enabling Remote adds `17322` without changing Local `17321`; disabling Remote
   removes only the Remote listener and does not stop sessions.
2. Remote refuses to start without a valid device credential and rejects public
   or wildcard binds unless the documented explicit safeguards are satisfied.
3. Every non-pairing Remote API and terminal operation rejects missing or invalid
   credentials, while Local loopback behavior remains compatible.
4. Device Token plaintext is displayed or delivered once, is absent from the
   database and logs, and is independently revocable.
5. QR pairing expires in 60 seconds, requires CLI/TUI confirmation, and never
   embeds a long-lived Token.
6. WebSocket tickets expire after 30 seconds, are single-use, and are bound to a
   device and managed session.
7. Revoking a device immediately disconnects its control connections without
   terminating Claude or Codex processes.
8. A second terminal viewer is read-only; takeover keeps the previous viewer
   connected but immediately removes its input permission.
9. Instance-ID changes and incompatible Remote protocol versions require explicit
   user action and cannot trigger a security downgrade.

### 13.4 Multi-backend GUI

1. Desktop stores multiple uniquely named backend profiles and connects to only
   the selected profile.
2. Local profiles accept loopback URLs only; Remote profiles require HTTPS,
   successful authentication, and backend identity confirmation before saving.
3. Remote credentials use the system credential store and never appear in
   `localStorage` or plaintext profile files.
4. Switching profiles does not stop sessions, does not silently fall back to
   Local, and restores the selected backend's last active session and navigation.
5. Network reconnect does not reload the GUI or discard terminal/UI state.
6. Active Remote profile refreshes do not overlap, late results from a previously
   selected profile are ignored, and identity changes stop automatic refresh
   until the user explicitly confirms or cancels them.
7. Switching Backend Profiles never renders a cached live session list from the
   previous backend, and late session operations cannot mutate the selected
   backend's Unified Sessions snapshot.

### 13.5 Android

1. Android 10+ can pair with a Remote backend using QR, manual Token, or clipboard
   import and persist the credential in Keystore.
2. Portrait and landscape layouts respect safe areas and software-keyboard viewport
   changes while keeping the terminal usable.
3. The terminal toolbar sends Esc, Tab, Ctrl combinations, arrows, and paste; IME,
   selection, copy, and reconnection are validated on a physical device.
4. Exiting the application disconnects the client but leaves managed sessions running.
5. CI produces signed arm64 and universal APK artifacts without exposing the
   signing key or backend credentials.
6. Product text states that background notifications are best-effort until push
   delivery is implemented.

## 14. Validation

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
Claude or Codex credentials in CI. Browser validation covers desktop layout,
terminal connection, create, switch, resize, restart, and close flows.

Remote security tests must additionally cover:

- Local and Remote listener isolation and lifecycle
- exact Host and Origin validation without trusting forwarded headers
- missing, malformed, incorrect, valid, and revoked device Tokens
- Token digest persistence and scans proving plaintext never reaches storage or logs
- pairing expiry, reuse rejection, confirmation, cancellation, and concurrent claims
- WebSocket ticket expiry, single use, device/session binding, and reconnect issuance
- immediate revocation of active HTTP/WebSocket identities
- rate-limit backoff and recovery without permanent lockout
- control-lease acquisition, concurrent takeover ordering, read-only enforcement,
  previous-controller demotion, and 30-second recovery
- protocol-major rejection, capability downgrade, and backend instance-ID changes

Installed-client tests use fake credential-store adapters to verify that Remote
Tokens never enter WebView persistence or error output. Integration tests exercise
the platform credential implementation where CI supports it.

Android validation includes narrow touch-enabled browser tests plus Tauri Android
smoke builds. Release acceptance also requires physical Android 10+ device checks
for QR and deep links, Keystore persistence, portrait and landscape safe areas,
software keyboard and Chinese IME behavior, terminal shortcuts, selection and
paste, Wi-Fi/cellular reconnection, notification limitations, and application exit.
Signing secrets must be available only to protected release jobs and must not be
present in artifacts or logs.
