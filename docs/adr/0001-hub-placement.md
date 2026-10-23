# 0001 Hub placement

Status: Accepted, 2026-10-23

## Context

Something must own the message store, accept messages from agents, run the noise policy and drive the Mac UI and the phone relay. The Mac is the main surface; Linux follows later. The app is signed with a free Personal Team, so there is no paid entitlement or server component to lean on. Vaultday already runs a Rust core in-process behind SwiftUI over UniFFI.

## Decision

- The Rust core runs **in-process** in the SwiftUI menu-bar app, started at login with `SMAppService.mainApp`. The UI calls the core through UniFFI; there is no local HTTP hop between them.
- The same crates build **`gentboxd`**, a headless hub for Linux, for a Mac without the app, and for integration tests.
- **One hub per user**, enforced by an exclusive lock on `<state>/hub.lock`. A second hub refuses to start and names the holder.

## Consequences

- One process, one SQLite writer, no IPC between UI and core. The UI is also the only local answer path, which keeps agents from answering their own questions (architecture §8). Quitting the app stops the hub; messages sent meanwhile fail fast (connection refused) and senders fall back to their own prompts.
- Presence sensing (frontmost app, idle, lock, Focus) lives in Swift and is pushed into the core; the core stays pure and portable.
- Linux gets the full hub on day one, with a UI later (Electron on `gentbox-node`).
- A crash of the UI takes the hub down with it. Acceptable for a menu-bar app; `SMAppService` relaunches at login only, so the store (PR 2) runs SQLite in WAL mode and the hub re-arms timers for open messages at start.

## Alternatives

- **Separate daemon plus thin UI client on the Mac** (launchd agent + app over localhost). Survives UI crashes and lets the UI restart freely, but doubles the moving parts: a second signed binary, a launchd plist, IPC versioning, and two processes to keep alive under a Personal Team signature. Kept possible: `gentboxd` exists, and the FFI surface mirrors the HTTP API, so the app could later become a client.
- **Hub in the TypeScript CLI/MCP server.** Easy for agents, but every agent session would start its own hub, and the Mac UI would need a second runtime.
- **Hosted hub.** Rejected: messages contain code and commands, and the user wants no server to run.
