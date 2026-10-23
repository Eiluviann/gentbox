# 0004 Policy engine and classification

Status: Accepted, 2026-10-23. Superseded in part by [0007](0007-message-type-urgency-blocking.md): the policy is keyed on `(type, urgent, blocking)` instead of the class, and classification fills each field separately. The pure-function design, timers and delivery log stand.

## Context

The product is the noise policy: which message pings where, when, and how loudly. It has to be predictable (you can tell why something pinged), configurable per class, and testable without a Mac. Classification is declared by the sender plus deterministic rules; no model.

## Decision

- **Classification**: declared class → user rules (`rules.toml`, first match wins, `override = true` lets a user rule beat a declared class) → built-in rules (shipped, visible) → `generic`. Rules match on source, event glob, title regex, cwd glob and metadata; they set `class` and/or `mute`.
- **Policy as a pure function** in `notifier-core`: `decide(message, context, policy) -> DeliveryPlan`. Context carries presence, Focus, quiet hours, the conversation's open items and the current time; no IO, no clock reads.
- **Timers re-decide.** Re-ping, phone escalation (silent-then-ping), snooze and expiry are scheduled by the hub actor; when one fires it calls `decide` again with the current context, so stale pings never go out.
- **Every decision is recorded** (`deliveries` table) with the rule and policy row that produced it, shown in the UI as "why this pinged".
- **Defaults** are the table in `docs/architecture.md` §6.2, each cell configurable per class in `config.toml` (per source: Planned).

## Consequences

- The whole policy is covered by table-driven Rust tests: one row per class × presence × Focus × quiet hours, plus one per coalescing rule. No UI needed to test noise behaviour.
- The engine cannot learn; tuning is by editing rules and per-class settings. That is the point: you can always predict it.
- Presence and Focus quality limit the policy's quality. Focus detection depends on a Shortcuts automation (Spike S3).

## Alternatives

- **LLM classification**: rejected by decision (cost, latency, unpredictability, sending content to a model).
- **Rules in a scripting language (Rhai, JS)**: more power, but untestable user logic and a sandbox to maintain. TOML match tables cover the known shapes.
- **Policy inside the Swift app**: would make Linux re-implement it and leave it without fast tests.
