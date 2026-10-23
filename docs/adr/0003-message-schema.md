# 0003 Message schema v1

Status: Accepted, 2026-10-23. Superseded in part by [0007](0007-message-type-urgency-blocking.md): `class` became `type`, `urgent` and `blocking`, and `resolves` became `rescinds`.

## Context

Every sender speaks one wire format. It must carry enough to classify without a model, to coalesce per conversation, to offer actions, and to bring the right session forward, while keeping phone payloads minimal.

## Decision

The schema is [`../schema/message-v1.json`](../schema/message-v1.json) (JSON Schema 2020-12), examples in [`../schema/examples/`](../schema/examples/). Key choices:

- **camelCase field names.** The clients that write it (hook, CLI, MCP server) are TypeScript under Biome's naming rule; Rust renames with `#[serde(rename_all = "camelCase")]`. Claude Code's snake_case hook payloads are translated by the hook, never passed through.
- **No sender-chosen message id.** The hub assigns a ULID and returns it; it is the only id in URLs and responses. The optional `idempotencyKey` only makes retries safe.
- **`version: 1` required**, `additionalProperties: false` at the top level and in nested objects, so a typo fails with 422 instead of being ignored. `metadata` (flat, ≤ 32 keys, scalar values) is the extension point.
- **`class` optional.** Declared wins; otherwise rules (ADR 0004). `event` is the sender's own dotted event name, the main thing rules match on.
- **`conversation.id` required.** Coalescing, supersede and resolve all work within a conversation; senders without sessions use a stable id per task.
- **`correlationKey`** names the slot a message occupies in its conversation (`permission`, `idle`): a newer message with the same key supersedes the open one in place. **`resolves`** closes other slots.
- **`awaitsResponse`** declares the intent to long-poll; the hub acts on the live waiter (a poll in flight or ended under 10 s ago), shown as answer buttons only while one exists.
- **Actions** `approve`, `deny`, `choice` (with `value`), `reply` (free text) are answers; `open` and `snooze` (minutes in `value`) are non-answers, never returned to the sender. `choice` is an addition to the plan's list, for multiple-choice questions (MCP `ask`, elicitation-like prompts) that `approve`/`deny` cannot express.
- **`sensitive`** forces the minimal phone payload whatever the setting. `body` reaches the phone only as an excerpt trimmed to the 4 KB APNs budget, never through ntfy (amended by ADR 0006).
- **`focus`** (`url`, `bundleId`, `terminal`) describes how to bring the conversation forward; the hub tries them in that order.
- The **`actionResponse`** returned by the long-poll is defined in the same file (`$defs/actionResponse`).
- Size caps: title 120, summary 400, body 64 KiB, 6 actions; request body 128 KiB.

## Consequences

- One file validates both directions; the Rust types and the TypeScript types are generated or hand-written against it and tested on the examples (PR 1 and PR 7 load `examples/*.json`).
- Additive optional fields stay in v1. Anything that breaks a valid v1 sender is v2 on `/v2/`.
- Strict validation means senders must upgrade deliberately; the 422 body names the failing JSON pointer.

## Alternatives

- **snake_case** to mirror Claude Code and ntfy: rejected for the lint friction in every TypeScript client.
- **Free-form payloads with server-side mapping only**: easier for senders, but rules would match on unstable shapes and validation errors would be silent.
- **Class required**: forces every sender to understand our taxonomy; rules exist so thin senders do not have to.
