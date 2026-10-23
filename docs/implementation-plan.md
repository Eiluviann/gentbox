# Implementation plan (Phase 3)

The plan's seven PRs, refined against [`architecture.md`](architecture.md). PR 2 is split in two (store, then hub, ingest and daemon), a `notifier-hub` crate is added so the crate graph stays acyclic, phone answers come back through an ntfy answer topic (so PR 5 needs no inbound listener), the Tailscale listener and pairing move into the now optional iOS PR, and each PR names the spike it must close first. PR 0 (repo bootstrap) is done.

Every PR passes, before merge:

```bash
cd core && cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features
```

```bash
pnpm typecheck && pnpm lint && pnpm test
```

and, from PR 3 on, `xcodebuild test` for the Apple targets (signing disabled in CI). Every PR gets a separate reviewer session (Opus, effort high) before merge, per the plan.

Model column: the implementing session's `/model` and effort, per the routing rules (judgment, security and concurrency on Opus; well-specified work on Sonnet).

## Overview

| PR | Title | Depends on | Model · effort | Spike |
| --- | --- | --- | --- | --- |
| 1 | `notifier-core`: domain, classification, policy engine | 0 | Sonnet · high | — |
| 2a | `notifier-store`: SQLite, migrations, retention | 1 | Sonnet · high | — |
| 2b | `notifier-hub` + `notifier-ingest` + `agent-notifierd`: hub actor, HTTP API, long-poll | 2a | Opus · high | — |
| 3 | `notifier-ffi` + macOS app shell: menu-bar inbox, login item, presence | 2b | Opus · high | — |
| 4 | Rich popups, native notifications, action round-trip, Focus endpoint | 3 | Sonnet · high | S3 |
| 5 | `notifier-relay` ntfy adapter, answer-topic subscription, action tokens, silent-then-ping | 2b; its Settings pane after 4 | Opus · high | S2 |
| 6 | Optional: iOS app + Tailscale remote listener + pairing | 5 | Opus · high (listener, pairing), Sonnet · high (app) | — |
| 7 | `agent-notify` CLI + stdio MCP server | 2b | Sonnet · medium | — |

PR 7 can run in parallel with 3 and 4 once 2b has merged (it only touches `cli/`). PR 5's Rust part (`notifier-relay`, token minting and redemption in `notifier-hub`) can too; its Mac Settings pane waits for PR 4, since both edit `apps/apple/AgentNotifier/`.

## PR 1 · `notifier-core`

**Scope.** The pure domain from architecture §1, §5, §6, §7: types, validation of semantic rules the JSON Schema cannot express, classification, the policy function, lifecycle transitions (including snooze), the coalescing decisions (supersede, fold, dedupe, completion, progress) as functions of the conversation's open items and their live-waiter flags, and action-token signing/verification over bytes. No IO, no async, no clock.

**Files.**

- `core/crates/notifier-core/src/lib.rs` (module map, re-exports)
- `message.rs` (`Message`, `MessageClass`, `Source`, `Conversation`, `FocusTarget`, `Action`, `ActionKind`, serde camelCase)
- `lifecycle.rs` (`LifecycleState`, `ResolutionReason`, the transition function returning `Result`)
- `classify.rs` (`Rule`, `RuleSet` parsed from TOML, `classify(&Message, &RuleSet, &BuiltInRules) -> Classification` with the class source)
- `builtin_rules.toml` (the Claude Code table of architecture §5.3, `include_str!`)
- `policy.rs` (`Policy`, `PolicyContext`, `Presence`, `DeliveryPlan`, `decide`)
- `coalesce.rs` (`CoalesceOutcome`: new, superseded, folded, duplicate; completion, progress and `resolves` effects)
- `action_token.rs` (payload encoding, HMAC-SHA256 sign and constant-time verify; the nonce ledger is the store's)
- `tests/` table tests; `tests/schema_examples.rs` deserialises every `docs/schema/examples/*.json`
- Dependencies: `serde`, `serde_json`, `toml`, `globset`, `regex`, `thiserror`, `time` (types only), `hmac`, `sha2`, `subtle`, `base64`.

**Tests.** Table-driven, one row each:

- Classification: declared wins; user rule; `override = true` beats declared; built-in rule; fallback `generic`; `mute`; invalid TOML is an error value, not a panic.
- Policy: every class × {present, away, locked} × {Focus off/on} × {quiet hours off/on}; blocking breakthrough setting; silent-then-ping (silent at once while the Mac is in use, escalation after 2 min if unaddressed, immediate when away); frontmost suppression of completions.
- Coalescing: idempotency key; supersede in place; no re-ping unless louder; a message without a live waiter folds into one with a live waiter (same key, and blocking-into-blocking); dedupe within 10 s; completion resolves blocking items without a live waiter and leaves `action_nonblocking` alone; progress resolves `idle` and blocking items without a live waiter; `resolves`; parallel `permission:<agent>:<hash>` keys coexist.
- Lifecycle: every allowed transition (including acknowledged → open when a snooze ends) and that every other one is rejected; `open` and `snooze` never resolve; the lifecycle → `actionResponse.status` table of architecture §5.2.
- Action tokens: valid, expired, wrong message, wrong action, tampered payload, tampered MAC.

**Acceptance.** `cargo test -p notifier-core` green; the examples deserialise; `decide` and `classify` are free of `std::time::SystemTime::now`, IO and `unsafe` (a test greps the crate source).

## PR 2a · `notifier-store`

**Scope.** SQLite persistence for messages, conversations, actions, responses, deliveries and the action-token ledger; migrations; retention sweep.

**Files.** `core/crates/notifier-store/{Cargo.toml, src/lib.rs, src/migrations/0001_init.sql, src/repository.rs, src/retention.rs}`, `tests/`. `rusqlite` with `bundled`, WAL mode, `foreign_keys = ON`.

**Tests.** Round-trip every core type; open messages listed by conversation and class; supersede chain recorded; retention deletes only resolved/expired older than the cutoff; token nonce insert is unique (second insert reports reuse); migration from an empty file and re-open of an existing file.

**Acceptance.** Store API returns `Result` for not-found and constraint outcomes and only throws (panics) on broken invariants; temp-dir tests; no SQL outside the crate.

**Model note.** Sonnet: the first migration has no user data to lose. Later migrations that transform data go to Opus.

## PR 2b · `notifier-hub` + `notifier-ingest` + `agent-notifierd`

**Scope.** Architecture §4, §5, §7, §8, §9.1: the hub actor in `notifier-hub` (commands, live-waiter tracking, timers for re-ping, phone escalation, snooze and expiry, dispatch to a `dyn Relay` and a UI delegate), the axum router on the Unix socket and loopback, sender-token auth, the challenge-response `hello`, `Host` check on TCP only, schema validation, rate limits, idempotency, long-poll with `watch`, `progress`, discovery files, the hub lock, the daemon binary. No answer route on the local router.

**Files.**

- `core/crates/notifier-hub/src/{lib.rs, actor.rs, commands.rs, waiters.rs, timers.rs, dispatch.rs}`
- `core/crates/notifier-ingest/src/{lib.rs, local_routes.rs, auth.rs, hello.rs, limits.rs, long_poll.rs, discovery.rs}`
- `core/crates/agent-notifierd/src/main.rs` (config, lock, tracing, SIGTERM)
- `core/tests/http/*.sh` or Rust integration tests with `reqwest`
- The schema is embedded with `include_str!("../../../../docs/schema/message-v1.json")` and validated with `jsonschema`.

**Tests.**

- 401 without or with a wrong token; 400 on a foreign `Host` over TCP and no check on the socket; `hello` proof verifies and never needs the token; 413; 422 with the JSON pointer; 429 with `Retry-After`; no route answers or lists messages with the sender token.
- One message per class through the real router; store state after each.
- Long-poll: `pending` after `wait`; `answered` when an answer (injected through the hub handle, as the UI does) lands mid-wait; two concurrent answers → one accepted, one 409; `superseded`, `resolved` and `expired` wake the waiter; the live waiter lapses 10 s after the last poll; snooze and open never wake it.
- Second hub on the same state folder exits non-zero naming the lock holder.
- Discovery: `token` 0600, folder 0700, `port` removed on clean shutdown.

**Acceptance.** `agent-notifierd` runs on macOS and Linux CI; the `curl` script in the plan's verification section posts one message per class and gets the expected classes back.

## PR 3 · `notifier-ffi` + macOS app shell

**Scope.** UniFFI bindings (`Hub` object: start, stop, inbox queries, answer, presence updates; `HubDelegate` callback interface for delivery plans and state changes), the XCFramework build script, the `NotifierCore` Swift package, the XcodeGen project with Vaultday's signing setup, the menu-bar extra with the inbox list, login item, and presence sensing (frontmost app, idle, lock) pushed into the core.

**Files.**

- `core/crates/notifier-ffi/{Cargo.toml (own [lints] without unsafe_code), src/lib.rs, uniffi.toml}`
- `apps/apple/{project.yml, Signing.xcconfig, Signing.personal-team.xcconfig, Signing/*.entitlements, scripts/build-xcframework.sh}`
- `apps/apple/NotifierCore/` (Swift package wrapping the generated bindings)
- `apps/apple/AgentNotifier/` (app: `MenuBarInbox`, `PresenceMonitor`, `LoginItem`)
- `docs/signing.md` adapted from Vaultday's (bundle id `com.eiluviann.agentnotifier`, App Group `<TEAM>.com.eiluviann.agentnotifier`)
- `.github/workflows/ci.yml`: an Apple job building and testing with signing disabled

**Tests.** Rust tests for the FFI surface (no Swift needed); XCTest for `PresenceMonitor` mapping; XCUITest smoke: app launches, menu-bar inbox shows a message posted with `curl`.

**Acceptance.** A Personal Team build runs from the login item, receives a `curl`-posted message and lists it; the Mac app embeds no provisioning profile unless an entitlement requires it (check `Contents/embedded.provisionprofile`; architecture §11).

## PR 4 · Popups, native notifications, round-trip

**Scope.** Architecture §6.2 Mac column, §7.7, §9.1, §9.2, §6.3: non-activating `NSPanel` persistent popups (stack of 3, "+N waiting"), transient toasts, `UNUserNotificationCenter` categories (approve, deny, reply with text input, open, snooze), sounds, re-ping, the "why this pinged" line, the open action (URL allowlist, then app activation), `POST /v1/presence/focus` and the `agentnotifier://focus` URL handler.

**Spike S3 first.** Automatic detection is preferred: check on macOS 26 that the active Focus can be read from `~/Library/DoNotDisturb/DB/` with Full Disk Access, and that changes can be watched. If it works, ship that reader with a one-time Full Disk Access prompt; if not, confirm that Shortcuts on macOS 26 offers a "When Focus turns on/off" automation that can run `agent-notify focus on|off` and ship that setup guide instead. Record the result in architecture §11.

**Files.** `apps/apple/AgentNotifier/{Popups/, Toasts/, Notifications/, Actions/, Focus/}`.

**Tests.** XCUITest: blocking popup appears, persists across app switches, resolves on Approve and the waiting `curl` long-poll returns `answered`; completion toast suppressed when the conversation's app is frontmost; superseding message updates the popup in place without a second sound. Unit tests for notification category mapping.

**Acceptance.** The plan's Mac verification items pass; answering from a native notification and from the popup both reach the waiting sender; late second answer shows "Already answered".

## PR 5 · `notifier-relay` + phone answers

**Scope.** Architecture §6.2 (phone column), §9.3, §10, §12: the `Relay` trait, the ntfy adapter (publish, update by sequence id, clear, retries, the answer-topic subscription with reconnect), the notification content (title phrases, summary, icon attachments: agent icons by URL, project icons uploaded), payload modes (`summary`, `minimal`, `sensitive`), silent-then-ping escalation, token minting and redemption in `notifier-hub` against the store's nonce ledger, topics and credentials in the Keychain, and the Mac Settings pane for the phone (ntfy.sh or self-hosted, QR code to subscribe the ntfy app, rotate topics).

**Spike S2 first.** On the user's iPhone and Watch with the ntfy app and ntfy.sh: an `http` button posting to an answer topic works; a priority-2 message arrives silently; republishing the same sequence id at priority 5 re-alerts (else: clear and publish anew); `clear` removes it; an image attachment shows as the thumbnail; buttons appear on the Watch and work there. Record the results in architecture §11; if delivery is unreliable, decide with the user whether to switch to the Pushover adapter (ADR 0005) before going on.

**Files.** `core/crates/notifier-relay/src/{lib.rs, relay.rs, ntfy.rs, answers.rs, content.rs, icons.rs, payload.rs}`, `core/crates/notifier-hub/src/{action_tokens.rs, escalation.rs}`, a mock ntfy server in `tests/`, `apps/apple/AgentNotifier/Settings/Phone*`.

**Tests.** Title phrase per class and action set (URGENT prefix only for blocking; Goal complete only for `scope: goal`); summary fallback to title; `minimal` and `sensitive` carry only the phrase and no answer button; `body` and `cwd` never sent; silent-then-ping: silent at once when the Mac is active, escalation after 2 min only if unaddressed, immediate audible when away, 60-s shared sound; priority mapping; retries with backoff against the mock; answer subscription reconnects; tokens: valid, expired, wrong message, wrong action, tampered, replayed (ignored and logged), constant-time compare used.

**Acceptance.** End to end with the user's phone on ntfy.sh: a blocking message arrives silently while the Mac is in use and pings 2 min later; Approve on the phone reaches the waiting sender through the answer topic; replaying the same token does nothing; with the Mac locked it pings at once.

## PR 6 · Optional: iOS app over Tailscale

Only if the user wants full content and text replies on the phone; it needs Tailscale on the Mac and the iPhone.

**Scope.** The remote router in `notifier-ingest` bound to the Tailscale address (off by default; refuses the hub's own node), device pairing by QR code with revocable device tokens, inbox, detail and answer routes for paired devices; the iOS app (inbox, detail, reply and choice answers, deep link from the ntfy notification's click URL) over HTTPS with the MagicDNS certificate; Personal Team signing for iOS (7-day profile, weekly reinstall documented in `docs/signing.md`).

**Files.** `core/crates/notifier-ingest/src/remote/{listener.rs, routes.rs, pairing.rs}`, `apps/apple/AgentNotifierPhone/`, the iOS target in `project.yml`.

**Tests.** Listener refuses to bind to anything but a 100.64.0.0/10 address or the configured MagicDNS name; device token issue, use and revocation; XCTest for the API client against recorded fixtures; XCUITest smoke for inbox and detail.

**Acceptance.** A paired iPhone lists open messages, opens one from the ntfy notification, and sends a reply that reaches the waiting sender.

## PR 7 · `agent-notify` CLI + stdio MCP server

**Scope.** TypeScript (ESM, strict) package `cli/`: discovery with the `hello` proof and socket-first transport (§8), `agent-notify send|ask|wait|resolve|progress|focus on|off` (no `answer` or `list`: the sender token cannot do either, by design), and a stdio MCP server with `notify` (class `action_nonblocking` or `generic`, explicit) and `ask` (sends with `awaitsResponse`, waits up to a timeout, returns the answer or "no answer"). Types written against the schema and checked against the examples.

**Files.** `cli/src/{discovery.ts, hub-client.ts, message-types.ts, commands/*.ts, mcp-server.ts, main.ts}`, Vitest tests next to each.

**Tests.** Vitest: discovery order, socket preference and the `hello` proof (a wrong proof never sends the token); request building per command; the examples parse into the TS types; MCP tool schemas; `ask` timeout path. One integration test against `agent-notifierd` (skipped when the binary is absent).

**Acceptance.** `agent-notify send --class completion --title …` reaches the Mac app; the MCP server answers `tools/list` and an `ask` round-trips through the app.

## After Phase 3: Phase 4 inputs

- **Spike S1** (first task of Phase 4): in a terminal CLI session, does Claude Code show its permission prompt while a `PermissionRequest` hook is still running? It decides whether the hook may wait long when you are at the terminal (architecture §11).
- Hook mapping is fixed in architecture §5.3, including the async `UserPromptSubmit`/`PostToolUse`/`PostToolUseFailure` progress calls and the hook resolving its own message whenever it stops waiting.
- Packaging as a Claude Code plugin (hooks + MCP), merged with the existing user hooks, never replacing them.
