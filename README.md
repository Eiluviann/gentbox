# Gentbox

Receives notifications from LLM agents (Claude Code first) and pings you only with what needs you: a blocked session waiting for an answer, a finished task, a question that can wait. Everything else lands silently in an inbox. Rust core and a native menu-bar app on macOS; v1 is desktop only. Later: an iOS app with actionable push on the iPhone and Apple Watch (sent from your Mac to Apple with your own APNs key; ntfy carries the answers and is the fallback without a paid Apple team), and a Linux client.

Bring your own keys: it runs on your machines and uses services you pick with your own credentials (an APNs key from your own Apple team, ntfy, optionally Pushover or Tailscale). The project hosts nothing for anyone; you can self-host parts for yourself (ntfy, for example), but nothing requires a server.

Every message has a type (`decision_required`, `action_required`, `task_completed`, `generic`), an urgency (`urgent` true or false) and a blocking level (`non_blocking`, `deferrable`, `blocking`). The sender declares them or deterministic rules fill them in; no model is involved. Agents can also withdraw (rescind) their own notifications.

## Layout

- `core/` Cargo workspace. `gentbox-core` is a placeholder until Phase 3. Planned: `gentbox-store`, `gentbox-ingest`, `gentbox-relay`, `gentbox-ffi`, the `gentboxd` binary, and `gentbox-node` (napi-rs).
- `cli/` `@gentbox/cli`, placeholder. Planned: the `gentbox` CLI and the `gentbox-mcp` stdio MCP server.
- `apps/apple/` placeholder. Planned: the macOS menu-bar app and the iOS app (XcodeGen).
- `docs/` [architecture](docs/architecture.md), [ADRs](docs/adr/), the [message schema v1](docs/schema/message-v1.json) with examples, and the [implementation plan](docs/implementation-plan.md).

## Commands

```bash
pnpm install && pnpm typecheck && pnpm lint && pnpm test
```

```bash
cd core && cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features
```
