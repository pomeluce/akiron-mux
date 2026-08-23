---
status: accepted
---

# Replace the Desktop Client with GPUI

Replace the Tauri-based Desktop Client with one GPUI and GPUI Component implementation in a single feature-complete delivery. Keep the React application as the Embedded WebUI, but do not ship or maintain Tauri and GPUI as alternative desktop clients; the cutover occurs only after the GPUI client matches the existing desktop behavior and passes the agreed terminal, performance, security, packaging, and platform gates.

## Considered Options

- Maintain Tauri and GPUI desktop clients in parallel.
- Ship a reduced GPUI prototype before desktop feature parity.
- Complete GPUI behind the existing release boundary, then replace Tauri once.

## Consequences

Development keeps Tauri as a private comparison baseline until cutover, while the delivered change must also replace its native integrations, release packaging, and desktop test coverage. The Embedded WebUI remains React-based, so client behavior and backend protocols must stay consistent across two UI implementations even though only one is an installed Desktop Client.
