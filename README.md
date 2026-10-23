# Agent Notifier

Receives notifications from LLM agents (Claude Code first) and pings you only with what needs you: a blocked session waiting for an answer, a finished task, a question that can wait. Everything else lands silently in an inbox. Rust core, a SwiftUI menu-bar app on macOS, an iOS app with actionable push on the iPhone and Apple Watch (sent from your Mac to Apple with your own APNs key; ntfy carries the answers and is the fallback without a paid Apple team), a Linux client later.

Bring your own keys: it runs on your machines and uses services you pick with your own credentials (an APNs key from your own Apple team, ntfy, optionally Pushover or Tailscale). The project hosts nothing for anyone; you can self-host parts for yourself (ntfy, for example), but nothing requires a server.

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
