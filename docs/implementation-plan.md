# Implementation plan (Phase 3)

The plan's seven PRs, refined against [`architecture.md`](architecture.md). PR 2 is split in two (store, then hub, ingest and daemon), a `gentbox-hub` crate is added so the crate graph stays acyclic, the phone path follows ADR 0006 (paid Apple team, chosen 2026-10-23): our own iOS app over APNs is core, ntfy is the answer channel plus the delivery fallback, Tailscale is an optional extra PR, and each PR names the spike it must close first. PR 0 (repo bootstrap) is done.

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
| 1 | `gentbox-core`: domain, classification, policy engine | 0 | Sonnet · high | — |
| 2a | `gentbox-store`: SQLite, migrations, retention | 1 | Sonnet · high | — |
| 2b | `gentbox-hub` + `gentbox-ingest` + `gentboxd`: hub actor, HTTP API, long-poll | 2a | Opus · high | — |
| 3 | `gentbox-ffi` + macOS app shell: menu-bar inbox, login item, presence, paid-team signing | 2b | Opus · high | — |
| 4 | Rich popups, Time Sensitive native notifications, action round-trip, Focus via `INFocusStatusCenter` | 3 | Sonnet · high | S3 |
| 5 | `gentbox-relay`: APNs adapter, ntfy answer channel and fallback delivery, action tokens, pairing registrations, silent-then-ping | 2b; its Settings pane after 4 | Opus · high | S2 |
| 6 | iOS app (core): inbox, notification categories, Service Extension, optional Content Extension, pairing | 5 | Opus · high (pairing, extensions, crypto), Sonnet · high (inbox UI) | S2 |
| 7 | `gentbox` CLI + `gentbox-mcp` stdio MCP server | 2b | Sonnet · medium | — |
| 8 | Optional: Tailscale remote listener for full bodies and history | 6 | Opus · high | — |

PR 7 can run in parallel with 3 and 4 once 2b has merged (it only touches `cli/`). PR 5's Rust part (`gentbox-relay`, token minting and redemption in `gentbox-hub`) can too; its Mac Settings pane waits for PR 4, since both edit `apps/apple/Gentbox/`. PR 6 needs PR 5 merged and the paid membership active (enrolment can take a day or two); until then the ntfy fallback carries the phone.

## PR 1 · `gentbox-core`

**Scope.** The pure domain from architecture §1, §5, §6, §7: types, validation of semantic rules the JSON Schema cannot express, classification, the policy function, the `(type, urgent, blocking)` model with per-field classification and normalization (ADR 0007), lifecycle transitions (including snooze and defer), the coalescing decisions (supersede, fold, dedupe, and the implicit rescinds by completion and progress) as functions of the conversation's open items and their live-waiter flags, and action-token signing/verification over bytes. No IO, no async, no clock.

**Files.**

- `core/crates/gentbox-core/src/lib.rs` (module map, re-exports)
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

**Acceptance.** `cargo test -p gentbox-core` green; the examples deserialise; `decide` and `classify` are free of `std::time::SystemTime::now`, IO and `unsafe` (a test greps the crate source).

**Topics and threads (ADRs 0008, 0009).** The topic and thread models (`kind` hints, `implicit`) and the resolution order: `thread.id` fixes both (conflicting `topic` → error); topic: named → rule → the conversation's latest default thread's topic → the agent's General topic; thread: named key (create on first use) → rule → the conversation's default thread in that topic (implicit, created on first use); the thread takeover by the first explicit `kind: "conversation"` thread; rule templates with `{project}`, `{agent}`, `{conversation}`; effective mute (topic or thread) in the policy (non-urgent → inbox only, `shouldWait: false`; urgent unaffected); topics and threads never coalesce. Tests: every resolution branch, a thread spanning two conversations, a conversation posting into two topics (one default thread per topic), implicit General per agent, takeover (idempotent, only once, never by the inline form), rule precedence, `kind` never changing a decision, every `(type, urgent, blocking)` combination muted by topic, by thread, and not muted.

**Title guard (ADR 0010).** `title_guard.rs`: the pure check over title, summary, conversation, topic and thread titles, and the rewrite (original into the body as a code box, summary's first sentence or the fallback phrase per type and project, key-to-words for inline topics and threads). Tests: a table of at least 40 strings, each heuristic positive and negative (flags vs hyphenated words, paths vs `and/or` and bare file names, versions, URLs, JSON, `npm login` vs "Cargo tests pass"), every rewrite branch, and that no rewritten title or summary ever fails the guard.

## PR 2a · `gentbox-store`

**Scope.** SQLite persistence for messages, conversations, actions, responses, deliveries and the action-token ledger; migrations; retention sweep.

**Files.** `core/crates/gentbox-store/{Cargo.toml, src/lib.rs, src/migrations/0001_init.sql, src/repository.rs, src/retention.rs}`, `tests/`. `rusqlite` with `bundled`, WAL mode, `foreign_keys = ON`.

**Tests.** Round-trip every core type; open messages listed by conversation, type, urgency and blocking level; rescinded messages kept with their reason; supersede chain recorded; retention deletes only answered, dismissed, rescinded and expired messages older than the cutoff; token nonce insert is unique (second insert reports reuse); migration from an empty file and re-open of an existing file.

**Acceptance.** Store API returns `Result` for not-found and constraint outcomes and only throws (panics) on broken invariants; temp-dir tests; no SQL outside the crate.

**Model note.** Sonnet: the first migration has no user data to lose. Later migrations that transform data go to Opus.

**Topics and threads (ADRs 0008, 0009).** Tables `topics` (nullable unique key, kind, implicit, status, style, mute, mark-read, created-by) and `threads` (topic id, nullable key unique per topic, kind, implicit, status, closed-by-topic, mute, mark-read, created-by), `conversation_default_threads (conversation, topic) → thread`, and on messages a non-null `thread_id` plus a denormalised `topic_id`, all in the first migration. Tests: upserts by key; the takeover moves messages and deletes the implicit thread in one transaction; moving a thread moves its messages' `topic_id`; mark read per topic and per thread; closing a topic closes its threads and reopening reopens only those; retention keeps anything with messages and deletes closed or implicit empty topics and threads after the retention period.

## PR 2b · `gentbox-hub` + `gentbox-ingest` + `gentboxd`

**Scope.** Architecture §4, §5, §7, §8, §9.1: the hub actor in `gentbox-hub` (commands, live-waiter tracking, timers for re-ping, phone escalation, snooze and expiry, dispatch to a `dyn Relay` and a UI delegate), the axum router on the Unix socket and loopback, sender-token auth, the challenge-response `hello`, `Host` check on TCP only, schema validation, rate limits, idempotency, long-poll with `watch`, `progress`, discovery files, the hub lock, the daemon binary. No answer route on the local router.

**Files.**

- `core/crates/gentbox-hub/src/{lib.rs, actor.rs, commands.rs, waiters.rs, timers.rs, dispatch.rs}`
- `core/crates/gentbox-ingest/src/{lib.rs, local_routes.rs, auth.rs, hello.rs, limits.rs, long_poll.rs, discovery.rs}`
- `core/crates/gentboxd/src/main.rs` (config, lock, tracing, SIGTERM)
- `core/tests/http/*.sh` or Rust integration tests with `reqwest`
- The schema is embedded with `include_str!("../../../../docs/schema/message-v1.json")` and validated with `jsonschema`.

**Tests.**

- 401 without or with a wrong token; 400 on a foreign `Host` over TCP and no check on the socket; `hello` proof verifies and never needs the token; 413; 422 with the JSON pointer; 429 with `Retry-After`; no route answers or lists messages with the sender token.
- Every example in `docs/schema/examples/` through the real router (one per type and per blocking level); store state after each; `DELETE` and the conversation `rescind` route wake waiters with `rescinded`; Defer wakes them with `deferred`.
- Long-poll: `pending` after `wait`; `answered` when an answer (injected through the hub handle, as the UI does) lands mid-wait; two concurrent answers → one accepted, one 409; `superseded`, `resolved` and `expired` wake the waiter; the live waiter lapses 10 s after the last poll; snooze and open never wake it.
- Second hub on the same state folder exits non-zero naming the lock holder.
- Discovery: `token` 0600, folder 0700, `port` removed on clean shutdown.

**Acceptance.** `gentboxd` runs on macOS and Linux CI; the `curl` script in the plan's verification section posts one message per type and blocking level and gets the expected fields back.

**Topics and threads (ADRs 0008, 0009).** Topic routes (`POST /v1/topics`, `GET /v1/topics`, `GET`/`PATCH /v1/topics/{id}`, `POST /v1/topics/{id}/close|reopen|rescind`), thread routes (`POST /v1/threads`, `GET /v1/threads`, `GET`/`PATCH /v1/threads/{id}`, `POST /v1/threads/{id}/close|reopen|rescind`, `PUT /v1/threads/{id}/topic`), `PUT /v1/messages/{id}/thread`, and inline resolution in `POST /v1/messages`. Tests: concurrent upserts of one key create one topic or thread; 201 vs 200; listings return no messages; close with and without `rescindOpenMessages` (waiters get `rescinded`); a message to a closed thread reopens it and its topic; `topicId` and `threadId` in the 201 response; `PATCH` on implicit topics and threads answers 409; a thread move into a topic with the same key answers 409.

**Title guard (ADR 0010).** The pipeline runs the guard after classification; `warnings` in the 201 response (`$defs/ingestResponse`); the per-source `titleGuard = "reject"` strict mode answering 422 `title_looks_like_command`; 422 on explicit topic and thread creates or PATCHes with a flagged title. Tests: a command title is rewritten with a warning and the original is first in the body; strict mode rejects it; a flagged conversation title is ignored.

## PR 3 · `gentbox-ffi` + macOS app shell

**Scope.** UniFFI bindings (`Hub` object: start, stop, inbox queries, answer, presence updates; `HubDelegate` callback interface for delivery plans and state changes), the XCFramework build script, the `GentboxCore` Swift package, the XcodeGen project with signing for the paid team (Vaultday's xcconfig layout, the shipped identifiers), the menu-bar extra with the inbox list, login item, and presence sensing (frontmost app, idle, lock) pushed into the core.

**Files.**

- `core/crates/gentbox-ffi/{Cargo.toml (own [lints] without unsafe_code), src/lib.rs, uniffi.toml}`
- `apps/apple/{project.yml, Signing.xcconfig, Signing.personal-team.xcconfig, Signing/*.entitlements, scripts/build-xcframework.sh}`
- `apps/apple/GentboxCore/` (Swift package wrapping the generated bindings)
- `apps/apple/Gentbox/` (app: `MenuBarInbox`, `PresenceMonitor`, `LoginItem`)
- `docs/signing.md`: the paid team as the default (bundle id `com.eiluviann.gentbox`, App Group `group.com.eiluviann.gentbox`, 1-year development profiles, Time Sensitive and Communication Notifications on the Mac), and Vaultday's Personal Team recipe kept as the fallback for builds without a membership (ntfy delivery only)
- `.github/workflows/ci.yml`: an Apple job building and testing with signing disabled

**Tests.** Rust tests for the FFI surface (no Swift needed); XCTest for `PresenceMonitor` mapping; XCUITest smoke: app launches, menu-bar inbox shows a message posted with `curl`.

**Acceptance.** A paid-team build runs from the login item, receives a `curl`-posted message and lists it, and its embedded profile grants Time Sensitive and Communication Notifications; a Personal Team build (`Signing.personal-team.xcconfig`) still builds and runs without them.

**Topics and threads (ADRs 0008, 0009).** FFI inbox queries grouped topic → thread → messages (conversation as row metadata); mute, mark read, close and reopen at both levels; move a thread or a message. The menu-bar inbox shows topic and thread header rows and a Closed section. Tests: XCUITest for grouping, General, and muting by topic and by thread.

## PR 4 · Popups, native notifications, round-trip

**Scope.** Architecture §6.2 Mac column, §7.7, §9.1, §9.2, §6.3: non-activating `NSPanel` persistent popups (stack of 3, "+N waiting"), transient toasts, `UNUserNotificationCenter` categories (approve, deny, reply with text input, open, snooze) with Time Sensitive for blocking items, the notification content of architecture §10 (title phrase, subtitle, summary, icon thumbnail), sounds, re-ping, the "why this pinged" line, the open action (URL allowlist, then app activation), Focus from `INFocusStatusCenter` (authorisation prompt, polling on presence changes), and for Personal Team builds the fallbacks `POST /v1/presence/focus` and the `gentbox://focus` URL handler.

**Spike S3 first.** On macOS 26 with the paid-team build: `INFocusStatusCenter.requestAuthorization` prompts, `focusStatus.isFocused` turns true when a Focus without Gentbox on its allowed list is on and false otherwise, and how fast it updates when polled. If it fails on the Mac, fall back to the `~/Library/DoNotDisturb/DB/` reader behind Full Disk Access, then to the Shortcuts automation. Record the result in architecture §11.

**Files.** `apps/apple/Gentbox/{Popups/, Toasts/, Notifications/, Actions/, Focus/}`.

**Tests.** XCUITest: blocking popup appears, persists across app switches, resolves on Approve and the waiting `curl` long-poll returns `answered`; completion toast suppressed when the conversation's app is frontmost; superseding message updates the popup in place without a second sound. Unit tests for notification category mapping.

**Acceptance.** The plan's Mac verification items pass; answering from a native notification and from the popup both reach the waiting sender; late second answer shows "Already answered".

**Topics and threads (ADRs 0008, 0009).** Topic and thread chips on popups and toasts; native notifications use the thread as thread identifier and "topic · thread" in the subtitle; topic and thread detail views. Tests: a muted topic or thread shows no popup for a non-urgent blocking message and still shows an urgent one.

## PR 5 · `gentbox-relay`: APNs, ntfy answers and fallback

**Scope.** Architecture §6.2 (phone column), §9.3, §10, §12 and ADR 0006 (Rust side):

- **`apns` adapter:**
  - ES256 JWT from the `.p8` key (header `kid`, claims `iss`, `iat`), cached and refreshed between 20 and 60 min, never more often.
  - One HTTP/2 connection per environment (`api.push.apple.com`, `api.sandbox.push.apple.com`) with `apns-topic` (the bundle id; unrelated to Gentbox and ntfy topics), `apns-push-type: alert`, `apns-priority` 10 or 5, `apns-expiration` and `apns-collapse-id`.
  - Payload building under the 4,096-byte cap: the body excerpt is trimmed last, UTF-8 safe.
  - Response handling: drop the device token on 410 `Unregistered` or `ExpiredToken`; on 400 `BadDeviceToken` or `DeviceTokenNotForTopic`, retry once against the other environment, then drop; refresh the JWT on 403 `ExpiredProviderToken`; back off on 429 and 5xx.
  - Background pushes to remove resolved notifications.
- **`ntfy` adapter:** the ntfy answer-topic subscription (action tokens and device registrations, reconnect with backoff), and fallback delivery (publish, update by sequence id, clear, icon attachments) used until a device is paired or when APNs has no valid token.
- **Content and policy:** title phrases, subtitle, summary, icon references, payload modes (`summary`, `minimal`, `sensitive`), silent-then-ping (passive, then the same collapse id at the audible level).
- **Hub side (`gentbox-hub`):** token minting and redemption against the store's nonce ledger; device registry with environment, pairing-secret HMAC check and revocation.
- **Settings › Phone pane on the Mac:** APNs key (`.p8`, key id, team id, stored in the Keychain and shown masked), ntfy server and token, the pairing QR code, devices, delivery channel choice, payload mode.

**Spike S2 first** (needs the membership active; runs with a minimal test app or the PR 6 skeleton):

- A collapsed replacement at `time-sensitive` alerts again after a `passive` push.
- A background push can remove a delivered notification, and how reliably.
- Background action handling can post to ntfy before iOS suspends the app.
- Watch: actions and text reply work there, and whether the Service Extension's image shows.

The ntfy app's behaviour (silent priority 2, update by sequence id, thumbnails) is re-checked for the fallback. Record the results in architecture §11.

**Files.** `core/crates/gentbox-relay/src/{lib.rs, relay.rs, apns/{client.rs, jwt.rs, payload.rs, responses.rs}, ntfy/{publish.rs, answers.rs}, content.rs, icons.rs}`, `core/crates/gentbox-hub/src/{action_tokens.rs, devices.rs, escalation.rs}`, mock APNs (HTTP/2) and mock ntfy servers in `tests/`, `apps/apple/Gentbox/Settings/Phone*`.

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
- Approve from the lock screen asks for Face ID and reaches the waiting sender through the ntfy answer topic.
- Replaying the token does nothing.
- With the ntfy fallback selected, the same flow works through the ntfy app.

**Topics and threads (ADRs 0008, 0009).** APNs `thread-id` = the message's thread id (implicit included); subtitle topic · thread without repeats and without "General"; on the ntfy fallback "topic · thread" leads the body. Tests: payload thread id and subtitle across explicit, implicit and General cases; a muted topic or thread produces no push.

## PR 6 · iOS app (core)

**Scope.** ADR 0006, iOS side:

- **The app:** inbox and detail from received pushes (stored in the App Group container); pairing by QR code (posts its device token, environment and version, HMAC-signed, to the ntfy answer topic; re-registers when iOS rotates the token).
- **Notification categories** `approval`, `question`, `choice`, `action`, `completion`, `info`, each answerable one with a `-defer` variant: Approve with `.authenticationRequired`, Deny, Reply as text input, Done, Defer, Open. Background handlers post action tokens and reply text to the ntfy answer topic.
- **Notification Service Extension:** icon thumbnail from the App Group cache, fetching a missing project icon once from its ntfy attachment; optional decryption of end-to-end content; optional Communication Notifications avatar.
- **Optional Notification Content Extension:** markdown excerpt, project and conversation, dynamic choice buttons via `notificationActions`.
- **Signing:** paid team, Push Notifications and Time Sensitive capabilities, `aps-environment` development for Xcode builds and production for TestFlight.

**Files.** `apps/apple/GentboxPhone/` (app), `apps/apple/GentboxPhoneService/` (service extension), `apps/apple/GentboxPhoneContent/` (content extension), the iOS targets in `project.yml`, shared models in `GentboxCore` where possible (the iOS app does not embed the Rust hub).

**Tests.** XCTest:

- Payload decoding from the Rust payload fixtures, including trimmed and encrypted ones.
- Category and action mapping.
- The pairing message and its HMAC.
- The extension's icon cache, both hit and miss.

XCUITest smoke for inbox and detail.

**Acceptance.** The PR 5 acceptance flows pass on the user's iPhone and Watch, including text reply from the lock screen, choices from the expanded view, and the project icon as the thumbnail.

**Topics and threads (ADRs 0008, 0009).** Inbox and detail grouped topic → thread, notifications grouped by `thread-id`, mute and mark read at both levels in the app. Tests: XCTest for grouping from payload fixtures.

## PR 7 · `gentbox` CLI + `gentbox-mcp` stdio MCP server

**Scope.** TypeScript (ESM, strict) package `cli/`: discovery with the `hello` proof and socket-first transport (§8), `gentbox send|ask|wait|rescind|progress|focus on|off` (no `answer` or `list`: the sender token cannot do either, by design), and the `gentbox-mcp` stdio MCP server (a second bin in the same package) with `notify` (explicit `type`, `urgent`, `blocking`), `ask` (`decision_required` with `awaitsResponse`; `blocking` or `deferrable` chosen by the agent; waits up to a timeout and returns the answer, `deferred`, or "no answer") and `rescind`. Types written against the schema and checked against the examples.

**Files.** `cli/src/{discovery.ts, hub-client.ts, message-types.ts, commands/*.ts, mcp-server.ts, main.ts}`, Vitest tests next to each.

**Tests.** Vitest: discovery order, socket preference and the `hello` proof (a wrong proof never sends the token); request building per command; the examples parse into the TS types; MCP tool schemas; `ask` timeout path. One integration test against `gentboxd` (skipped when the binary is absent).

**Acceptance.** `gentbox send --type task_completed --title …` reaches the Mac app; the MCP server answers `tools/list` and an `ask` round-trips through the app.

**Topics and threads (ADRs 0008, 0009).** MCP tools `create_topic`, `close_topic`, `create_thread` (with `kind` and a flag to stand for the current conversation) and `close_thread`, and `topic` and `thread` parameters on `notify` and `ask`; CLI `gentbox topic …`, `gentbox thread create|close|reopen|list|move`, and `--topic`/`--thread` on `send` and `ask`. Tests: Vitest for request building and the inline shortcuts.

**Title guard (ADR 0010).** The `notify`, `ask`, `create_topic` and `create_thread` tool descriptions and `title` parameters state the rule with one good and one bad example, and the tool results pass the hub's `warnings` to the model verbatim. Tests: Vitest that warnings reach the tool result.

## PR 8 · Optional: Tailscale remote listener

Only if the user wants full bodies beyond the push excerpt and the inbox history on the phone; it needs Tailscale on the Mac and the iPhone.

**Scope.** The remote router in `gentbox-ingest` bound to the Tailscale address (off by default; refuses the hub's own node), read routes for full bodies and history, authenticated with the device's pairing credential; the iOS app loads them over HTTPS with the MagicDNS certificate when reachable and falls back to the excerpt otherwise.

**Files.** `core/crates/gentbox-ingest/src/remote/{listener.rs, routes.rs}`, `apps/apple/GentboxPhone/Remote/`.

**Tests.** Listener refuses to bind to anything but a 100.64.0.0/10 address or the configured MagicDNS name; revoked devices are refused; the app falls back cleanly when the Mac is unreachable.

## After Phase 3: Phase 4 inputs

- **Spike S1** (first task of Phase 4): in a terminal CLI session, does Claude Code show its permission prompt while a `PermissionRequest` hook is still running? It decides whether the hook may wait long when you are at the terminal (architecture §11).
- Title builders per hook event (architecture §5.3, ADR 0010), with Vitest tables: Bash with and without `description`, a `description` that itself fails the guard, Edit/Write paths reduced to file names, WebFetch hosts, MCP tool names in words, `Stop` titles from `last_assistant_message`; no raw `tool_input` ever in a title or summary, always in the body.
- Hook mapping is fixed in architecture §5.3, including the async `UserPromptSubmit`/`PostToolUse`/`PostToolUseFailure` progress calls and the hook resolving its own message whenever it stops waiting.
- Packaging as a Claude Code plugin (hooks + MCP), merged with the existing user hooks, never replacing them.
