# 0009 Threads under topics

Status: Accepted, 2026-10-23. Supersedes in part [0008](0008-topics.md): its implicit per-conversation topics and their takeover, the topics' `defaultFor`, the one-level inbox, and `thread-id` = topic. Topics themselves, their API, mute, close, rescind-all and rules stand.

## Context

The user: "On top of topic add a thread. Rule of a thumb: Topic - project, bigger undertaking, may be a generic category for simple conversations. Thread - a new version of the project, phase of the large implementation or simply a conversation topic".

## Decision

### Hierarchy: topic → thread → messages

- **Every message belongs to exactly one thread, and every thread to exactly one topic.** The conversation stays metadata on each message, for Open, focus and coalescing.
- **A thread may span conversations and agents.** Example: "Phase 3 · ingest" worked on by two sessions. A conversation may also post into several threads.
- **Topics stay as ADR 0008 defines them,** with `kind` hints `project`, `category`, `other`, or any short string. Implicit topics are now only the per-agent **General** topic (below); `kind: "conversation"` on a topic no longer means anything special, and topics have no takeover.
- **A thread** has:
  - `id`; `topicId`.
  - An optional agent-chosen `key`, unique within its topic, so the same key can exist under different topics.
  - `title`, optional `summary`, and a free-form `kind` hint (suggested `version`, `phase`, `conversation`, `other`; it never changes behaviour).
  - `implicit`, `status` (`open` or `closed`), `defaultFor` (the conversation it is the default thread of, if any), `createdBy`, and timestamps.
  - It shows its topic's `style`.
- **A message keeps its own `type`, `urgent` and `blocking`.** Coalescing and the implicit rescinds stay per conversation. Topics and threads only group.

### Resolving a message's topic and thread

1. **`thread: { "id": … }`:** that thread, and its topic. A `topic` that names another topic is a 422.
2. **Topic:** the first that applies:
   1. the named `topic`;
   2. a rule-assigned topic (for example, the project from the working directory);
   3. the topic of the conversation's most recent default thread, so a conversation stays where it was;
   4. the agent's implicit **General** topic: `implicit: true`, `kind: "general"`, no key, titled "General", created on first use. It is the "generic category for simple conversations".
3. **Thread, within that topic:** the first that applies:
   1. the named `thread: { "key": … }`, created on first use, titled `title` or the key;
   2. a rule-assigned thread;
   3. the conversation's **default thread in that topic**: an implicit thread (`kind: "conversation"`, no key, titled and renamed with the conversation), created on the conversation's first message that lands in that topic without naming a thread.

A conversation has at most one default thread per topic.

### Thread takeover

- **The trigger:** an agent creates an explicit thread with `kind: "conversation"` and `conversation.id` in some topic.
- **The effect:** it takes over that conversation's implicit thread in the same topic. Messages and mute state move in, the implicit thread is deleted, and later default messages of the conversation in that topic land in it. The response reports `defaultFor`.
- **Only the first takes over:** once the default thread is explicit, another `kind: "conversation"` thread is an ordinary one.
- **The inline `thread` form never takes over.**

### API (sender token)

| Method and path | Purpose |
| --- | --- |
| `POST /v1/threads` | Upsert by `key` within its topic (`threadCreate`; `topic` is a topicRef, resolved like a message's when absent): 201 new, 200 existing unchanged |
| `GET /v1/threads?topic={id}&status=open` | List threads: id, key, topic, title, kind, status, open-message and conversation counts; never messages |
| `GET /v1/threads/{id}` · `PATCH /v1/threads/{id}` | Read one; rename or edit (`threadUpdate`). Implicit threads answer 409 to agents |
| `POST /v1/threads/{id}/close` · `/reopen` · `/rescind` | Close (`closeRequest`, optionally withdrawing open messages), reopen, or withdraw its open messages |
| `PUT /v1/threads/{id}/topic` | Move a thread, with its messages, to another topic (`{ "topic": topicRef }`); 409 if its key exists there |
| `PUT /v1/messages/{id}/thread` | Move a message to another thread (`{ "thread": threadRef, "topic"?: topicRef }`), or back to its default resolution (`{ "thread": null }`). Replaces 0008's `PUT /v1/messages/{id}/topic` |

The topic endpoints of 0008 stand. `POST /v1/topics/{id}/close` also closes the topic's open threads; reopening the topic reopens the threads that closing it had closed. A message to a closed thread reopens it and, if needed, its topic.

- **On a message:** `thread` is `{ "id": … }` or `{ "key", "title"?, "summary"?, "kind"? }` (the inline shortcut), alongside the optional `topic` (same two forms).
- **MCP:** `create_thread` and `close_thread`, and a `thread` parameter next to `topic` on `notify` and `ask`. **CLI:** `gentbox thread create|close|reopen|list|move` and `--thread`.

### Rules

```toml
[[rule]]
name = "Project topic, one thread per conversation"
match = { source = "claude-code", cwd = "~/Developer/**" }
topic = { key = "project:{project}", title = "{project}", kind = "project" }
```

- **Placeholders:** `{project}` (the working-directory name), `{agent}` (the agent's display name) and `{conversation}` (the conversation title).
- **What a rule may set:** a `topic` template, a `thread` template (`thread = { key = "…", title = "…", kind = "…" }`), or both.
- **When it applies:** only to fields the message did not name.

### You, in the UI

- **Mute, mark read and close at both levels.**
  - Muting a topic covers all its threads, so a message is muted if its topic or its thread is muted.
  - Non-urgent messages in a muted topic or thread go to the inbox only, and their senders get `shouldWait: false`.
  - Urgent messages still come through.
- **Mark read** acknowledges every open message in the topic or thread.
- **Close** a topic or a thread, with the option to withdraw its open messages. Rescind-all works on a topic or a thread.
- **Move** a thread to another topic, or a message to another thread.

### Phone

- **Grouping:** APNs `thread-id` = the message's thread id, implicit threads included. Mac native notifications use the same thread identifier.
- **Subtitle:** blocking badge · topic title · thread title. A part is left out when it repeats the previous one (an implicit conversation thread named like its topic), and "General" is left out.
- **The topic on the stacked group:** not possible. The payload's `summary-arg` (`summaryArgument`) has been ignored since iOS 15 (deprecated, "summaryArgument is ignored"). `categorySummaryFormat` is not deprecated but can only use that argument and a count, so a category format such as "%u more" is the most a stack can say. The topic shows in the subtitle of each notification instead [Apple-summaryArgument, Apple-categorySummaryFormat].
- **The ntfy fallback** cannot group by Gentbox thread; "topic · thread" leads the notification text.

### Storage

- `threads` table: `topic_id`, nullable key (unique per topic), kind, implicit, status, closed-by-topic flag, mute flag, mark-read time, created-by.
- `conversation_default_threads (conversation, topic) → thread`.
- Messages: `thread_id` (never null) and a denormalised `topic_id`.
- Topics keep their mute and mark-read fields.
- Retention never deletes a thread or topic with messages. Closed or implicit empty ones go after the message retention period.

## Consequences

- **The inbox is two levels:** topic → thread → messages. A simple conversation with no topic and no rule shows as "General" → the conversation's thread.
- **Agents that never name anything still get one thread per conversation**, grouped per agent under General.
- **Moving a thread or a message** does not regroup notifications already delivered on the phone.
- **Each topic carries a short list of its threads,** which agents can list.

## Alternatives

- **Topics only (0008):** one level cannot say "project → phase".
- **Threads without topics:** loses the project level the user asked for.
- **Arbitrary nesting:** two levels match the user's rule of thumb; deeper trees complicate mute, close and the phone.
- **A per-conversation implicit topic (0008's default):** in the two-level model the conversation is a thread, and simple conversations share a General topic, as the user described.

## Amendments

- **2026-10-24: a muted message reports the mute and stays open.** Senders of non-urgent messages in a muted topic or thread still get `shouldWait: false`, and now also the warning `muted` naming the topic or thread. The message stays open in the inbox; only an explicit rescind from the agent withdraws it (ADR 0007, amendments).
- **2026-10-24: no close while items are open.** The user's decision during the PR 3 review. A topic or thread closes only when nothing in it is open (open, acknowledged, deferred or snoozed); otherwise the hub answers 409 `has_open_messages` with the count. `rescindOpenMessages: true` withdraws only the requester's own open messages first, and the close is refused, withdrawing nothing, when anything else stays open. An agent's General topic never closes (409 `general_topic`); its threads close under the same rule. On the Mac, nothing the user does withdraws a message: Close on a thread with open items shows them ("N items still open"), and items are answered, done or dismissed, several at once with bulk Dismiss, before the close. This replaces "open messages stay unless `rescindOpenMessages: true`" above.

## Sources

- [Apple-summaryArgument] UNNotificationContent.summaryArgument (deprecated iOS 15.0, "summaryArgument is ignored"): https://developer.apple.com/documentation/usernotifications/unnotificationcontent/summaryargument
- [Apple-categorySummaryFormat] UNNotificationCategory.categorySummaryFormat (iOS 12+, not deprecated): https://developer.apple.com/documentation/usernotifications/unnotificationcategory/categorysummaryformat
