# AkironMux

AkironMux manages configuration and live coding sessions for Claude Code and
Codex. This glossary defines the language used across the product.

## Configuration

**Configuration Catalog**:
The API Providers and their available Model Profiles or models configured for an
Agent.
_Avoid_: Agent Configuration

**Agent Configuration**:
The API Provider, applicable model selection, and Switching Mode currently
effective for an Agent. It may be selected through AkironMux or discovered from
the Agent's native configuration. The Agent's native configuration is the source
of truth for what is effective; AkironMux's stored active selection is a
rebuildable projection of it.
_Avoid_: Configuration Catalog

**Agent**:
An executable AI coding tool whose conversations AkironMux can start, resume,
and organize. The supported Agents are Claude Code and Codex.
_Avoid_: API Provider, Provider

**API Provider**:
A configured API endpoint, credential source, and model catalog used by an
Agent.
_Avoid_: Agent

**Model Profile**:
A named model selection belonging to an API Provider.
_Avoid_: Profile

**Local Switching Mode**:
The Claude configuration mode that applies an API Provider and Model Profile
directly to Claude Code's settings.
_Avoid_: Local

**Proxy Switching Mode**:
The Claude configuration mode that routes requests through the AkironMux proxy.
_Avoid_: Proxy

## Sessions

**Managed Session**:
A running Agent process and interactive terminal owned by the Session Backend.
Its lifetime is independent of any connected client.
_Avoid_: Native History Session, terminal

**Terminal Connection**:
A live channel between one client terminal surface and one Managed Session. It
can observe terminal output and may hold that Managed Session's Control Lease.
_Avoid_: Managed Session, Control Lease

**Native History Session**:
A persisted Agent conversation that AkironMux discovers in Claude Code or Codex
history. It can be resumed as a Managed Session but is not live-process state.
_Avoid_: Managed Session, history item

**Native History Ingestion**:
The incremental process that discovers Agent-native session and usage files and
updates AkironMux's rebuildable Native History index.
_Avoid_: Native History Session, live session synchronization

**Unified Sessions**:
The shared AkironMux view of Claude Code and Codex sessions through one list,
workspace, API, and UI. Its live client state belongs to exactly one selected
Backend Profile at a time. The Agents do not share native conversation context.

**Native Title**:
A session title supplied by the Agent's native history. It supersedes the
temporary directory-based label used before the Agent provides a title.
_Avoid_: session label

## Workspace Organization

**Project**:
A user-created, persisted directory scope that owns Native History Sessions in
its root and descendants. Installed clients display a Project as a Workspace.
_Avoid_: Workspace outside client-facing copy

**General**:
The single persisted root directory for non-Project work and its child
directories.
_Avoid_: default Project, general Workspace

**Other**:
The navigational group for discovered Native History Sessions outside Projects
and General. Other directories are not scopes for creating Managed Sessions.
_Avoid_: external Project

**Workspace Organization**:
The classification of Native History Sessions and their directories into
Project, General, and Other scopes.
_Avoid_: Workspace

## Backend Connections

**Session Backend**:
The host service that owns Managed Sessions and exposes them to AkironMux
clients.
_Avoid_: client, Agent

**Backend Instance**:
One independently identified installation of the Session Backend to which a
client can connect.
_Avoid_: Backend Profile

**Backend Profile**:
A client-owned connection record that locates a Backend Instance and remembers
its expected identity and credential reference.
_Avoid_: Profile, Backend Instance

**Backend Profile Lifecycle**:
The client-owned process that validates, pairs, confirms identity, activates,
refreshes, reorders, and removes Backend Profiles while preserving Device
Credential security ordering.
_Avoid_: backend settings, connection form

**Local Backend**:
The client-facing name for a loopback-only connection to a Session Backend's
Local listener. It is an access mode, not a separate Backend Instance.
_Avoid_: Local

**Remote Backend**:
The client-facing name for an authenticated connection to a Session Backend's
Remote listener. It is an access mode, not a separate Backend Instance.
_Avoid_: Remote

**Backend Instance ID**:
The stable identity of a Backend Instance that a client pins to a Backend
Profile.
_Avoid_: Backend Profile ID

## Remote Access

**Device Credential**:
A long-lived, independently revocable host-control credential issued to one
installed client device for Remote Backend access.
_Avoid_: WebSocket Ticket, read-only token

**Pairing Request**:
A short-lived, single-use request through which a user authorizes issuance of a
Device Credential to a client.
_Avoid_: Device Credential, pairing code

**WebSocket Ticket**:
A short-lived, single-use authorization derived from a Device Credential for one
Managed Session's terminal connection.
_Avoid_: Device Credential

**Terminal Control Lease**:
The exclusive right held by one terminal connection to send input to a Managed
Session. Other connected viewers remain read-only.
_Avoid_: Device Credential, session ownership
