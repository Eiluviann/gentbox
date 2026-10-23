# 0007 Message type, urgency and blocking; rescind and defer

Status: Accepted, 2026-10-23. Supersedes in part [0003](0003-message-schema.md) (the `class` field and `resolves`) and [0004](0004-policy-engine.md) (the class-keyed defaults and the classification order).

## Context

The four classes (`completion`, `action_blocking`, `action_nonblocking`, `generic`) mixed three questions into one value: what the message asks of you, how soon it matters, and what the agent does meanwhile. The user split them:

> Notification type: decision required, action required, task completed, generic. Urgent: true/false. Blocking: non-blocking (agent is already working and factored in that it will be deferring work until response comes), deferrable (agent has paused but user is able to request deferral so the agent keeps working on tasks that do not require input), blocking (work cannot continue until you make a decision). Agents should be able to rescind notifications as well.

Schema v1 is unreleased, so v1 is amended in place; there is no v2.

## Decision

### Three fields

| Field | Values | Default |
| --- | --- | --- |
| `type` | `decision_required`, `action_required`, `task_completed`, `generic` | `generic` |
| `urgent` | `true`, `false` | `false` |
| `blocking` | `non_blocking`, `deferrable`, `blocking` | `non_blocking` |

Definitions (confirmed by the user, 2026-10-23: "The reading is correct."):

- **`decision_required`:** the agent needs your answer: approve or deny, pick a choice, reply. Example: a permission prompt.
- **`action_required`:** you must do something yourself outside the conversation, then tell the agent with Done. Examples: sign in, add an API key, review a pull request.
- **`task_completed`:** work finished; `scope: goal` marks the agent's overall objective.
- **`generic`:** everything else, kept for reference.

### Valid combinations

- **`task_completed` and `generic` are always `non_blocking`.**
- **`generic` is never urgent:** it is inbox-only, so urgency would contradict it.
- **`scope` belongs to `task_completed`.**

The schema rejects a declared message that breaks one of these (422). When rules fill a missing field and produce an invalid combination, the hub normalizes it: `blocking` becomes `non_blocking`, `urgent` becomes `false`. It records the normalization in the delivery log.

Valid: `decision_required` and `action_required` with any urgency and any blocking level (6 each); `task_completed` urgent or not (2); `generic` (1). 15 combinations in all.

### Classification, per field

Declared fields win, one field at a time. Missing fields are filled by user rules, then built-in rules, in order; each matching rule sets only the fields still missing. A user rule with `override = true` may also replace declared fields. Then defaults, then normalization.

```toml
[[rule]]
name = "Claude Code permission prompts are urgent at work"
match = { source = "claude-code", event = "claude-code.permission_request", cwd = "~/Work/**" }
urgent = true
override = true
```

### Defer and Snooze

- **Defer is an answer, sent to the agent.**
  - It exists only on `deferrable` messages: the hub adds a Defer button by itself, and the sender does not declare one.
  - The waiting sender gets status `deferred`, meaning "keep working on what needs no input".
  - The message stays open and becomes `non_blocking`: the popup closes and re-pings stop.
  - The sender keeps polling and still gets the real answer later.
  - Defer uses an action token like any answer, so it goes on the phone as a push button.
- **Snooze is local, never sent.** It hides the message and re-pings it after N minutes, and the agent learns nothing. It is offered on the Mac and in the iOS app, not as a push button (the 3 phone buttons go to answers).

### Rescind

An agent withdraws its own messages:

- `DELETE /v1/messages/{id}` with `{ "reason": … }`. With no body the reason is `resolved_elsewhere`.
- `POST /v1/conversations/{agent}/{id}/rescind` with `{ "reason": …, "correlationKeys": [ … ] }`.
- The `rescinds` field on any new message, same shape.

Reasons: `resolved_elsewhere` (handled in the session or elsewhere), `obsolete` (no longer needed), `superseded` (replaced).

Effects:

- **State:** terminal `rescinded` with its reason. A rescinded message can no longer be answered; a late tap gets "Withdrawn".
- **Long-poll:** `{ "status": "rescinded", "rescindReason": … }`.
- **Mac:** popup and toast removed, native notification removed (`removeDeliveredNotifications`). The inbox row stays, marked "Withdrawn" with the reason in words ("Handled in the session", "No longer needed"). Superseded rows instead fold into the message that replaced them.
- **iPhone (own app):**
  - A background push asks the app to call `removeDeliveredNotifications`. The app confirms through the answer topic.
  - With no confirmation after 60 s (iOS throttles background pushes), the hub replaces the notification through its collapse id with a passive "Withdrawn" version.
- **iPhone (ntfy fallback):**
  - The ntfy iOS app cannot update, clear or delete a delivered notification. Its Notification Service Extension handles only `message` and `poll_request` events and logs `message_delete`/`message_clear` as irrelevant, and its message model has no sequence id [ntfy-ios ntfyNSE/NotificationService.swift, ntfy/Persistence/Notification.swift, commit 96e659c, 2026-06-26].
  - So a withdrawn message stays in Notification Centre, and its buttons, if tapped, are ignored by the hub (the token's message is closed).
  - If the original had already pinged audibly, the hub publishes one min-priority (silent) "Withdrawn: <title phrase>" notice so you are not left acting on it.
  - The same limitation means fallback escalation sends a second notification instead of updating the silent one.
- **Safety:** rescinding can never answer. The sender token can rescind but not approve; a rescinded permission prompt simply falls back to the agent's own prompt.

### Implicit rescinds

- **Supersede** (same correlation key) rescinds the old message with reason `superseded`, replacing it in place.
- **A `task_completed` message** rescinds the conversation's open `blocking` and `deferrable` messages that have no live waiter, with reason `obsolete`. `non_blocking` ones survive.
- **Conversation progress** (`POST …/progress`) rescinds the conversation's `blocking` and `deferrable` messages without a live waiter, and its `idle` item, with reason `resolved_elsewhere`.
- **A hook that stops waiting** deletes its own message with reason `resolved_elsewhere`. This replaces `POST /v1/messages/{id}/resolve`.

### Policy key

`decide` is keyed on `(type, urgent, blocking)`:

- **`blocking` drives persistence and escalation:** popup or not, re-ping cadence, whether Defer appears.
- **`urgent` drives interruption:**
  - Time Sensitive on the Mac and the phone.
  - Breaking through quiet hours and Focus.
  - The "URGENT:" title prefix (**to confirm**: the user's earlier rule prefixed blocking items; with urgency now its own field, the prefix follows `urgent`).
- **`type` picks the surface and the title phrase:**

| `type` | Title phrase |
| --- | --- |
| `decision_required` | Your input is needed |
| `action_required` | Action required |
| `task_completed` | Task complete (`scope: goal`: Goal complete) |
| `generic` | the message title (inbox only) |

The defaults table is in `architecture.md` §6.2.

### Claude Code mapping

Permission prompts become `decision_required` + `blocking` + not urgent. That settles the open question of giving permissions their own type: the type is the decision, the blocking level says the agent waits. Making them urgent is one rule (example above). The full mapping is in `architecture.md` §5.3.

## Consequences

- **Every policy row is a combination of three small enums:** 15 valid cells, tested exhaustively in PR 1.
- **A blocking prompt is no longer loud by default** (Time Sensitive, quiet-hours breakthrough). It persists and escalates, but only urgent items cut through Focus and quiet hours. That is the user's split, and a rule can mark any source or project urgent.
- **Defer needs a sender that understands `deferred`:** the Claude Code hook cannot defer a permission prompt, so it declares `blocking`, not `deferrable`. The MCP `ask` tool offers `deferrable`.
- **Rescinds are reliable on the Mac, best-effort on the own iPhone app** (with a guaranteed visible fallback), **and impossible on the ntfy fallback**.

## Alternatives

- **Keep four classes and add more:** the combinations would multiply into names (`urgent_blocking_decision` …).
- **A single `priority` number:** it hides whether the agent is waiting, which is what decides persistence and Defer.
- **Snooze as an agent-visible answer:** it would overlap Defer and tell the agent something you did not mean to say.
