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

An agent withdraws its own messages. Withdrawn means the agent asked: nothing else withdraws a message on its behalf, not a timeout, a cancel, a muted topic or a missing answer channel (amended 2026-10-24).

- `DELETE /v1/messages/{id}` with `{ "reason": … }`. With no body the reason is `resolved_elsewhere`.
- `POST /v1/conversations/{agent}/{id}/rescind` with `{ "reason": …, "correlationKeys": [ … ] }`.
- The `rescinds` field on any new message, same shape.

Reasons: `resolved_elsewhere` (handled in the session or elsewhere), `obsolete` (no longer needed), `superseded` (replaced).

Effects:

- **State:** terminal `rescinded` with its reason. A rescinded message can no longer be answered; a late tap gets "Withdrawn".
- **Long-poll:** `{ "status": "rescinded", "rescindReason": … }`.
- **Mac:** popup and toast removed, native notification removed (`removeDeliveredNotifications`). The inbox row stays, marked "Withdrawn" with the reason in words ("Handled in the session", "No longer needed"). Superseded rows instead fold into the message that replaced them.
- **iPhone (own app):**
  - A background push asks the app to call `removeDeliveredNotifications`. The app confirms through the ntfy answer topic.
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
- **A hook that stops waiting** deletes its own message with reason `resolved_elsewhere`. This replaces `POST /v1/messages/{id}/resolve`. (Amended 2026-10-24, see Amendments: a sender that stops waiting no longer rescinds.)

### Policy key

`decide` is keyed on `(type, urgent, blocking)`:

- **`blocking` drives persistence and escalation:** popup or not, re-ping cadence, whether Defer appears, and its own **blocking badge**, independent of the title prefix: "Waiting on you" for `blocking`, "Paused · can defer" for `deferrable`, none for `non_blocking`. An urgent blocking item shows both the prefix and the badge; a non-urgent blocking item shows only the badge.
- **`urgent` drives interruption:**
  - Time Sensitive on the Mac and the phone.
  - Breaking through quiet hours and Focus.
  - The "URGENT:" title prefix, and nothing else sets it (confirmed by the user, 2026-10-23: "The URGENT: prefix follows the urgent flag only; blocking shows its own badge").
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

## Amendments

- **2026-10-24: urgency is a badge, never a title prefix.** Asked "Should Mac banners and popups still start their title with URGENT:?", the user chose "Badge only". This supersedes the 2026-10-23 line above that made "URGENT:" the title prefix (kept for history). Urgency now shows as:
  - the red urgent badge under the type icon, in popups, toasts, the inbox, the steps view and the iOS app;
  - Time Sensitive delivery;
  - quiet-hours and Focus breakthrough;
  - sorting above non-urgent items.

  Native notifications, which have no badge slot, lead the subtitle with "Urgent ·", before the blocking badge ("Urgent · Waiting on you · topic · thread"). The type's title phrase is never changed by urgency.
- **2026-10-24: withdrawn means the agent asked.** The user decided: "Withdrawn is only to be used when the model asks to withdraw the notification. The model should be notified that their notification came through but it is muted." This supersedes the line above that had a hook delete its own message when it stops waiting (kept for history):
  - A sender that stops waiting (deadline, cancel, error, no answer channel) does **not** rescind. The waiter lapses after 10 s and the message becomes an ordinary inbox item (architecture §5.2).
  - A message delivered into a muted topic or thread (or muted by a rule) stays open, and the ingest response carries the warning `muted` so the sender knows it arrived without a ping. `shouldWait` stays `false`.
  - The rescind rules above and the implicit rescinds are otherwise unchanged; the label of the implicit ones is decided next.
- **2026-10-24: implicit clean-ups show as "Handled in the session", never "Withdrawn".** Asked whether the clean-ups from completion and progress should keep marking rows "Withdrawn", the user answered: "Keep the clean-up, new label." The implicit rescinds stay, with a new rescind reason, `handled_in_session`, that only the hub records:
  - A `task_completed` message and a progress call rescind the items described in §Implicit rescinds as `handled_in_session` (they were `obsolete` and `resolved_elsewhere`). Supersede keeps `superseded` and folds into its successor.
  - A sender cannot name `handled_in_session`: the rescind request, the `rescinds` field and the close request accept only `resolved_elsewhere`, `obsolete` and `superseded` (422 otherwise). An explicit rescind keeps exactly the reason it was given.
  - Labels: `handled_in_session` reads "Handled in the session", every other reason reads "Withdrawn". Withdrawn means the agent asked.
  - The store keeps `resolved_elsewhere` plus a marker column (migration 3), because the CHECK on `rescind_reason` cannot change without rebuilding the table. Rows closed before the migration keep their reason and read as withdrawn.
- **2026-10-24: a parked message is exempt from the implicit clean-ups.** The clean-up rule above (completion and progress clear items without a live waiter) would clear a muted ask the sender was told to collect later, or an ask that timed out, as "Handled in the session" although nobody handled it. The user decided: "a muted or timed-out ask the agent was told to collect later is exempt from the implicit clean-up; it closes only by my answer, dismiss, the agent's rescind or expiry."
  - The hub *parks* a message (`parked_at` on the message, `parked` in the ingest response): at ingest when it returns the `muted` warning for a message that awaits a response, and when the sender calls `POST /v1/messages/{id}/park` after a timeout (sender token, owner check).
  - A cancel, a failure, or a lapsed waiter does not park, so the stale prompt of a killed hook still clears.
  - Supersede by the same sender with the same correlation key is unchanged.
