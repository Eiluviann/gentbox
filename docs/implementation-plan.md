# Implementation plan (Phase 3)

The plan's seven PRs, refined against [`architecture.md`](architecture.md). PR 2 is split in two (store, then hub, ingest and daemon), a `notifier-hub` crate is added so the crate graph stays acyclic, the phone path follows ADR 0006 (paid Apple team, chosen 2026-10-23): our own iOS app over APNs is core, ntfy is the answer channel plus the delivery fallback, Tailscale is an optional extra PR, and each PR names the spike it must close first. PR 0 (repo bootstrap) is done.

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
| 3 | `notifier-ffi` + macOS app shell: menu-bar inbox, login item, presence, paid-team signing | 2b | Opus · high | — |
| 4 | Rich popups, Time Sensitive native notifications, action round-trip, Focus via `INFocusStatusCenter` | 3 | Sonnet · high | S3 |
| 5 | `notifier-relay`: APNs adapter, ntfy answer channel and fallback delivery, action tokens, pairing registrations, silent-then-ping | 2b; its Settings pane after 4 | Opus · high | S2 |
| 6 | iOS app (core): inbox, notification categories, Service Extension, optional Content Extension, pairing | 5 | Opus · high (pairing, extensions, crypto), Sonnet · high (inbox UI) | S2 |
| 7 | `agent-notify` CLI + stdio MCP server | 2b | Sonnet · medium | — |
| 8 | Optional: Tailscale remote listener for full bodies and history | 6 | Opus · high | — |

PR 7 can run in parallel with 3 and 4 once 2b has merged (it only touches `cli/`). PR 5's Rust part (`notifier-relay`, token minting and redemption in `notifier-hub`) can too; its Mac Settings pane waits for PR 4, since both edit `apps/apple/AgentNotifier/`. PR 6 needs PR 5 merged and the paid membership active (enrolment can take a day or two); until then the ntfy fallback carries the phone.

## PR 1 · `notifier-core`

**Scope.** The pure domain from architecture §1, §5, §6, §7: types, validation of semantic rules the JSON Schema cannot express, classification, the policy function, the `(type, urgent, blocking)` model with per-field classification and normalization (ADR 0007), lifecycle transitions (including snooze and defer), the coalescing decisions (supersede, fold, dedupe, and the implicit rescinds by completion and progress) as functions of the conversation's open items and their live-waiter flags, and action-token signing/verification over bytes. No IO, no async, no clock.

**Files.**

- `core/crates/notifier-core/src/lib.rs` (module map, re-exports)
- `message.rs` (`Message`, `MessageClass`, `Source`, `Conversation`, `FocusTarget`, `Action`, `ActionKind`, serde camelCase)
- `lifecycle.rs` (`LifecycleState`, `ResolutionReason`, the transition function returning `Result`)
- `classify.rs` (`Rule`, `RuleSet` parsed from TOML, `classify(&Message, &RuleSet, &BuiltInRules) -> Classification` with the source of each field: declared, rule name, default, normalized)
- `builtin_rules.toml` (the Claude Code table of architecture §5.3, `include_str!`)
- `policy.rs` (`Policy`, `PolicyContext`, `Presence`, `DeliveryPlan`, `decide`)
- `coalesce.rs` (`CoalesceOutcome`: new, superseded, folded, duplicate; completion, progress and explicit rescind effects)
- `rescind.rs` (`RescindReason`, rescind by id or correlation keys, the effects list consumed by the hub)
- `action_token.rs` (payload encoding, HMAC-SHA256 sign and constant-time verify; the nonce ledger is the store's)
- `tests/` table tests; `tests/schema_examples.rs` deserialises every `docs/schema/examples/*.json`
- Dependencies: `serde`, `serde_json`, `toml`, `globset`, `regex`, `thiserror`, `time` (types only), `hmac`, `sha2`, `subtle`, `base64`.

**Tests.** Table-driven, one row each:

- Field combinations: all 15 valid `(type, urgent, blocking)` combinations accepted; every invalid one (`task_completed`/`generic` with `deferrable` or `blocking`, urgent `generic`, `scope` on another type) rejected when declared and normalized when filled by rules, with the normalization recorded.
- Classification, per field: declared wins field by field; a rule fills only missing fields; later rules fill what earlier ones left; `override = true` replaces declared fields; built-in rules after user rules; defaults `generic`/`false`/`non_blocking`; `mute`; invalid TOML is an error value, not a panic; the Claude Code table of architecture §5.3 row by row.
- Policy: each of the 15 combinations × {present, away, locked} × {Focus off/on} × {quiet hours off/on}, checking popup vs banner vs toast vs inbox, re-ping cadence by blocking level, Defer offered only on `deferrable`, Time Sensitive and the URGENT prefix only when urgent, quiet-hours and Focus breakthrough only when urgent; silent-then-ping (silent at once while the Mac is in use, escalation after 2 min if unaddressed, immediate when away); frontmost suppression of `task_completed` toasts.
- Coalescing: idempotency key; supersede in place; no re-ping unless louder; a message without a live waiter folds into one with a live waiter (same key, and blocking-into-blocking); dedupe within 10 s; `task_completed` rescinds `blocking` and `deferrable` items without a live waiter as `obsolete` and leaves `non_blocking` ones alone; progress rescinds `idle` and `blocking`/`deferrable` items without a live waiter as `resolved_elsewhere`; parallel `permission:<agent>:<hash>` keys coexist.
- Rescind: by id and by correlation keys, each reason; via the `rescinds` field; a rescinded message rejects later answers; rescinding never produces an answer; the effects list (popup, toast, native notification, inbox "Withdrawn", phone removal or fallback notice) per channel and state.
- Defer: only on `deferrable`; turns `blocking` into `non_blocking` and re-runs the policy; the waiter gets `deferred` once and `pending` after; a later real answer still closes the message.
- Lifecycle: every allowed transition (including acknowledged → open when a snooze ends) and that every other one is rejected; `open` and `snooze` never close a message or reach the sender; the lifecycle → `actionResponse.status` table of architecture §5.2.
- Action tokens: valid, expired, wrong message, wrong action, tampered payload, tampered MAC.

**Acceptance.** `cargo test -p notifier-core` green; the examples deserialise; `decide` and `classify` are free of `std::time::SystemTime::now`, IO and `unsafe` (a test greps the crate source).

## PR 2a · `notifier-store`

**Scope.** SQLite persistence for messages, conversations, actions, responses, deliveries and the action-token ledger; migrations; retention sweep.

**Files.** `core/crates/notifier-store/{Cargo.toml, src/lib.rs, src/migrations/0001_init.sql, src/repository.rs, src/retention.rs}`, `tests/`. `rusqlite` with `bundled`, WAL mode, `foreign_keys = ON`.

**Tests.** Round-trip every core type; open messages listed by conversation, type, urgency and blocking level; rescinded messages kept with their reason; supersede chain recorded; retention deletes only answered, dismissed, rescinded and expired messages older than the cutoff; token nonce insert is unique (second insert reports reuse); migration from an empty file and re-open of an existing file.

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
- Every example in `docs/schema/examples/` through the real router (one per type and per blocking level); store state after each; `DELETE` and the conversation `rescind` route wake waiters with `rescinded`; Defer wakes them with `deferred`.
- Long-poll: `pending` after `wait`; `answered` when an answer (injected through the hub handle, as the UI does) lands mid-wait; two concurrent answers → one accepted, one 409; `superseded`, `resolved` and `expired` wake the waiter; the live waiter lapses 10 s after the last poll; snooze and open never wake it.
- Second hub on the same state folder exits non-zero naming the lock holder.
- Discovery: `token` 0600, folder 0700, `port` removed on clean shutdown.

**Acceptance.** `agent-notifierd` runs on macOS and Linux CI; the `curl` script in the plan's verification section posts one message per type and blocking level and gets the expected fields back.

## PR 3 · `notifier-ffi` + macOS app shell

**Scope.** UniFFI bindings (`Hub` object: start, stop, inbox queries, answer, presence updates; `HubDelegate` callback interface for delivery plans and state changes), the XCFramework build script, the `NotifierCore` Swift package, the XcodeGen project with signing for the paid team (Vaultday's xcconfig layout, the shipped identifiers), the menu-bar extra with the inbox list, login item, and presence sensing (frontmost app, idle, lock) pushed into the core.

**Files.**

- `core/crates/notifier-ffi/{Cargo.toml (own [lints] without unsafe_code), src/lib.rs, uniffi.toml}`
- `apps/apple/{project.yml, Signing.xcconfig, Signing.personal-team.xcconfig, Signing/*.entitlements, scripts/build-xcframework.sh}`
- `apps/apple/NotifierCore/` (Swift package wrapping the generated bindings)
- `apps/apple/AgentNotifier/` (app: `MenuBarInbox`, `PresenceMonitor`, `LoginItem`)
- `docs/signing.md`: the paid team as the default (bundle id `com.eiluviann.agentnotifier`, App Group `group.com.eiluviann.agentnotifier`, 1-year development profiles, Time Sensitive and Communication Notifications on the Mac), and Vaultday's Personal Team recipe kept as the fallback for builds without a membership (ntfy delivery only)
- `.github/workflows/ci.yml`: an Apple job building and testing with signing disabled

**Tests.** Rust tests for the FFI surface (no Swift needed); XCTest for `PresenceMonitor` mapping; XCUITest smoke: app launches, menu-bar inbox shows a message posted with `curl`.

**Acceptance.** A paid-team build runs from the login item, receives a `curl`-posted message and lists it, and its embedded profile grants Time Sensitive and Communication Notifications; a Personal Team build (`Signing.personal-team.xcconfig`) still builds and runs without them.

## PR 4 · Popups, native notifications, round-trip

**Scope.** Architecture §6.2 Mac column, §7.7, §9.1, §9.2, §6.3: non-activating `NSPanel` persistent popups (stack of 3, "+N waiting"), transient toasts, `UNUserNotificationCenter` categories (approve, deny, reply with text input, open, snooze) with Time Sensitive for blocking items, the notification content of architecture §10 (title phrase, subtitle, summary, icon thumbnail), sounds, re-ping, the "why this pinged" line, the open action (URL allowlist, then app activation), Focus from `INFocusStatusCenter` (authorisation prompt, polling on presence changes), and for Personal Team builds the fallbacks `POST /v1/presence/focus` and the `agentnotifier://focus` URL handler.

**Spike S3 first.** On macOS 26 with the paid-team build: `INFocusStatusCenter.requestAuthorization` prompts, `focusStatus.isFocused` turns true when a Focus without Agent Notifier on its allowed list is on and false otherwise, and how fast it updates when polled. If it fails on the Mac, fall back to the `~/Library/DoNotDisturb/DB/` reader behind Full Disk Access, then to the Shortcuts automation. Record the result in architecture §11.

**Files.** `apps/apple/AgentNotifier/{Popups/, Toasts/, Notifications/, Actions/, Focus/}`.

**Tests.** XCUITest: blocking popup appears, persists across app switches, resolves on Approve and the waiting `curl` long-poll returns `answered`; completion toast suppressed when the conversation's app is frontmost; superseding message updates the popup in place without a second sound. Unit tests for notification category mapping.

**Acceptance.** The plan's Mac verification items pass; answering from a native notification and from the popup both reach the waiting sender; late second answer shows "Already answered".

## PR 5 · `notifier-relay`: APNs, ntfy answers and fallback

**Scope.** Architecture §6.2 (phone column), §9.3, §10, §12 and ADR 0006 (Rust side):

- **`apns` adapter:**
  - ES256 JWT from the `.p8` key (header `kid`, claims `iss`, `iat`), cached and refreshed between 20 and 60 min, never more often.
  - One HTTP/2 connection per environment (`api.push.apple.com`, `api.sandbox.push.apple.com`) with `apns-topic`, `apns-push-type: alert`, `apns-priority` 10 or 5, `apns-expiration` and `apns-collapse-id`.
  - Payload building under the 4,096-byte cap: the body excerpt is trimmed last, UTF-8 safe.
  - Response handling: drop the device token on 410 `Unregistered` or `ExpiredToken`; on 400 `BadDeviceToken` or `DeviceTokenNotForTopic`, retry once against the other environment, then drop; refresh the JWT on 403 `ExpiredProviderToken`; back off on 429 and 5xx.
  - Background pushes to remove resolved notifications.
- **`ntfy` adapter:** the answer-topic subscription (action tokens and device registrations, reconnect with backoff), and fallback delivery (publish, update by sequence id, clear, icon attachments) used until a device is paired or when APNs has no valid token.
- **Content and policy:** title phrases, subtitle, summary, icon references, payload modes (`summary`, `minimal`, `sensitive`), silent-then-ping (passive, then the same collapse id at the audible level).
- **Hub side (`notifier-hub`):** token minting and redemption against the store's nonce ledger; device registry with environment, pairing-secret HMAC check and revocation.
- **Settings › Phone pane on the Mac:** APNs key (`.p8`, key id, team id, stored in the Keychain and shown masked), ntfy server and token, the pairing QR code, devices, delivery channel choice, payload mode.

**Spike S2 first** (needs the membership active; runs with a minimal test app or the PR 6 skeleton):

- A collapsed replacement at `time-sensitive` alerts again after a `passive` push.
- A background push can remove a delivered notification, and how reliably.
- Background action handling can post to ntfy before iOS suspends the app.
- Watch: actions and text reply work there, and whether the Service Extension's image shows.

The ntfy app's behaviour (silent priority 2, update by sequence id, thumbnails) is re-checked for the fallback. Record the results in architecture §11.

**Files.** `core/crates/notifier-relay/src/{lib.rs, relay.rs, apns/{client.rs, jwt.rs, payload.rs, responses.rs}, ntfy/{publish.rs, answers.rs}, content.rs, icons.rs}`, `core/crates/notifier-hub/src/{action_tokens.rs, devices.rs, escalation.rs}`, mock APNs (HTTP/2) and mock ntfy servers in `tests/`, `apps/apple/AgentNotifier/Settings/Phone*`.

**Tests.**

- **JWT:** header and claims; signature verifies against the public key; reused until 20 min; refreshed by 60 min; refreshed once on 403 `ExpiredProviderToken` and never in a loop; an invalid `.p8` is a `Result` error, not a panic.
- **Payload:**
  - Never exceeds 4,096 bytes, for a 64 KiB body and multi-byte text.
  - Title phrase per type ("URGENT: " only when urgent; Goal complete only for `scope: goal`); Time Sensitive only when urgent; `-defer` categories only on `deferrable`.
  - Summary falls back to the title.
  - `minimal` and `sensitive` carry only the phrase and no actions.
  - `cwd` is never present; the collapse id is 64 bytes or less.
- **Responses:** each status code maps to keep, retry, switch environment or drop; a 410 removes the device; 429 backs off.
- **ntfy:** answer envelopes parse; forged registration HMACs are rejected; reconnects after drops; fallback delivery used only when no APNs device is valid.
- **Silent-then-ping:** silent at once while the Mac is in use; escalation after 2 min only if unaddressed; immediate audible when away; the 60-s shared sound.
- **Tokens:** valid, expired, wrong message, wrong action, tampered, replayed (ignored and logged); constant-time compare.

**Acceptance.** Against the mock servers, every flow above passes. With the user's phone once PR 6 exists:

- A blocking message arrives silently while the Mac is in use and pings as Time Sensitive 2 min later.
- Approve from the lock screen asks for Face ID and reaches the waiting sender through the answer topic.
- Replaying the token does nothing.
- With the ntfy fallback selected, the same flow works through the ntfy app.

## PR 6 · iOS app (core)

**Scope.** ADR 0006, iOS side:

- **The app:** inbox and detail from received pushes (stored in the App Group container); pairing by QR code (posts its device token, environment and version, HMAC-signed, to the answer topic; re-registers when iOS rotates the token).
- **Notification categories** `approval`, `question`, `choice`, `action`, `completion`, `info`, each answerable one with a `-defer` variant: Approve with `.authenticationRequired`, Deny, Reply as text input, Done, Defer, Open. Background handlers post action tokens and reply text to the answer topic.
- **Notification Service Extension:** icon thumbnail from the App Group cache, fetching a missing project icon once from its ntfy attachment; optional decryption of end-to-end content; optional Communication Notifications avatar.
- **Optional Notification Content Extension:** markdown excerpt, project and conversation, dynamic choice buttons via `notificationActions`.
- **Signing:** paid team, Push Notifications and Time Sensitive capabilities, `aps-environment` development for Xcode builds and production for TestFlight.

**Files.** `apps/apple/AgentNotifierPhone/` (app), `apps/apple/AgentNotifierPhoneService/` (service extension), `apps/apple/AgentNotifierPhoneContent/` (content extension), the iOS targets in `project.yml`, shared models in `NotifierCore` where possible (the iOS app does not embed the Rust hub).

**Tests.** XCTest:

- Payload decoding from the Rust payload fixtures, including trimmed and encrypted ones.
- Category and action mapping.
- The pairing message and its HMAC.
- The extension's icon cache, both hit and miss.

XCUITest smoke for inbox and detail.

**Acceptance.** The PR 5 acceptance flows pass on the user's iPhone and Watch, including text reply from the lock screen, choices from the expanded view, and the project icon as the thumbnail.

## PR 7 · `agent-notify` CLI + stdio MCP server

**Scope.** TypeScript (ESM, strict) package `cli/`: discovery with the `hello` proof and socket-first transport (§8), `agent-notify send|ask|wait|rescind|progress|focus on|off` (no `answer` or `list`: the sender token cannot do either, by design), and a stdio MCP server with `notify` (explicit `type`, `urgent`, `blocking`), `ask` (`decision_required` with `awaitsResponse`; `blocking` or `deferrable` chosen by the agent; waits up to a timeout and returns the answer, `deferred`, or "no answer") and `rescind`. Types written against the schema and checked against the examples.

**Files.** `cli/src/{discovery.ts, hub-client.ts, message-types.ts, commands/*.ts, mcp-server.ts, main.ts}`, Vitest tests next to each.

**Tests.** Vitest: discovery order, socket preference and the `hello` proof (a wrong proof never sends the token); request building per command; the examples parse into the TS types; MCP tool schemas; `ask` timeout path. One integration test against `agent-notifierd` (skipped when the binary is absent).

**Acceptance.** `agent-notify send --type task_completed --title …` reaches the Mac app; the MCP server answers `tools/list` and an `ask` round-trips through the app.

## PR 8 · Optional: Tailscale remote listener

Only if the user wants full bodies beyond the push excerpt and the inbox history on the phone; it needs Tailscale on the Mac and the iPhone.

**Scope.** The remote router in `notifier-ingest` bound to the Tailscale address (off by default; refuses the hub's own node), read routes for full bodies and history, authenticated with the device's pairing credential; the iOS app loads them over HTTPS with the MagicDNS certificate when reachable and falls back to the excerpt otherwise.

**Files.** `core/crates/notifier-ingest/src/remote/{listener.rs, routes.rs}`, `apps/apple/AgentNotifierPhone/Remote/`.

**Tests.** Listener refuses to bind to anything but a 100.64.0.0/10 address or the configured MagicDNS name; revoked devices are refused; the app falls back cleanly when the Mac is unreachable.

## After Phase 3: Phase 4 inputs

- **Spike S1** (first task of Phase 4): in a terminal CLI session, does Claude Code show its permission prompt while a `PermissionRequest` hook is still running? It decides whether the hook may wait long when you are at the terminal (architecture §11).
- Hook mapping is fixed in architecture §5.3, including the async `UserPromptSubmit`/`PostToolUse`/`PostToolUseFailure` progress calls and the hook resolving its own message whenever it stops waiting.
- Packaging as a Claude Code plugin (hooks + MCP), merged with the existing user hooks, never replacing them.
