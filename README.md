# Agent Notifier

Receives notifications from LLM agents (Claude Code first) and pings you only with what needs you: a blocked session waiting for an answer, a finished task, a question that can wait. Everything else lands silently in an inbox. Rust core, SwiftUI menu-bar app on macOS and an iOS app over UniFFI, iPhone and Apple Watch push through ntfy, a Linux client later.

No hosted infrastructure: it runs on your machines and uses services you pick with your own keys (ntfy, Pushover, APNs, Tailscale). The project runs no server for anyone.

Every message carries one of four classes: `completion`, `action_blocking`, `action_nonblocking`, `generic`. The sender declares it or deterministic rules assign it; no model is involved.

## Layout

- `core/` Cargo workspace. `notifier-core` is a placeholder until Phase 3. Planned: `notifier-store`, `notifier-ingest`, `notifier-relay`, `notifier-ffi`, the `agent-notifierd` binary, and `notifier-node` (napi-rs).
- `cli/` `@agent-notifier/cli`, placeholder. Planned: the `agent-notify` CLI and stdio MCP server.
- `apps/apple/` placeholder. Planned: the macOS menu-bar app and the iOS app (XcodeGen).
- `docs/` [architecture](docs/architecture.md), [ADRs](docs/adr/), the [message schema v1](docs/schema/message-v1.json) with examples, and the [implementation plan](docs/implementation-plan.md).

## Commands

```bash
pnpm install && pnpm typecheck && pnpm lint && pnpm test
```

```bash
cd core && cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features
```
