# 0008 Topics

Status: Accepted, 2026-10-23 (revised the same day: topics are any grouping the agent chooses). Superseded in part by [0009](0009-threads.md): threads now sit under topics, a conversation's default is an implicit thread under an implicit per-agent General topic, topics no longer take over conversations, and `thread-id` follows the thread.

## Context

The user: "The model should also be able to create topics and associating messages with them to group notifications", and then: "The topic could be project, conversation or anything the model deems a separate grouping."

**Three things are called "topic" in this project. The docs keep them apart:**

- **Topic** (or Gentbox topic): this ADR's grouping.
- **ntfy topic:** ntfy's channel name. The docs always say "ntfy topic", for example the ntfy answer topic.
- **`apns-topic`:** the APNs request header that carries the app's bundle id.

## Decision

### A topic is whatever grouping the agent decides on

- **Examples:** a project, a conversation, or a release that spans an API session and a web session.
- **The `kind` hint:** an optional free-form string the UI may show, and nothing else. Suggested values are `project`, `conversation` and `other`; any other short agent string is accepted. It never changes behaviour.
- **Fields:**
  - `id` (hub ULID).
  - `key`, stable and agent-chosen, unique per hub across all agents. Implicit topics have none.
  - `title`, optional `summary`, `kind` and `style` (SF Symbol and colour hints, the symbol shown only when it is on the app's allowed list).
  - `implicit`, `status` (`open` or `closed`), `createdBy` (agent and conversation), and the `createdAt`, `updatedAt` and `closedAt` timestamps.
- **Flat:** topics are not nested.
- **Grouping only:** a message keeps its own `type`, `urgent` and `blocking`. Coalescing, supersede, fold and the implicit rescinds stay per conversation (architecture §7). The conversation stays on every message as metadata, for Open, focus and coalescing.

### Every message has exactly one topic; the inbox has one level

A message's topic is, in order:

1. the topic it names (`topic`);
2. a topic a rule assigns;
3. its conversation's **default topic**.

The default topic of a conversation is:

- **The implicit topic**, unless an explicit topic took over (below). It is created by the hub on the conversation's first topicless message:
  - `implicit: true`, `kind: "conversation"`, no key.
  - Titled with the conversation title, else the project name, and following conversation renames (`SessionStart`).
- **An explicit topic with `kind: "conversation"`**, once an agent creates one and links it to that conversation (`conversation.id` in `POST /v1/topics`). It **takes over**:
  - The implicit topic's messages move into it.
  - Its mute state carries over (muted if either was).
  - The implicit topic is deleted.
  - Later topicless messages of the conversation land in it.

  The `POST /v1/topics` response says so (`defaultFor`). Only the first such topic takes over: if the conversation's default is already explicit, a second `kind: "conversation"` topic is an ordinary topic. The takeover is idempotent through the key upsert.

So the inbox groups one level deep, **topic → messages**, and each message row shows its conversation.

### API (sender token)

| Method and path | Purpose |
| --- | --- |
| `POST /v1/topics` | Upsert by `key` (`topicCreate`): 201 with the new topic, or 200 with the existing one unchanged; takes over the conversation's implicit topic as above |
| `GET /v1/topics?status=open` | List topics, implicit ones included: id, key, title, kind, status, open-message count. Topics only, never their messages: agents need to find a topic to join it, but the sender token still cannot read messages (architecture §8) |
| `GET /v1/topics/{id}` | One topic (`topicRecord`) |
| `PATCH /v1/topics/{id}` | Rename, change the summary, kind or style (`topicUpdate`). Implicit topics are hub-managed: 409 for agents (create a `kind: "conversation"` topic to take over instead) |
| `POST /v1/topics/{id}/close` | Close (`topicClose`); open messages stay unless `rescindOpenMessages: true` |
| `POST /v1/topics/{id}/reopen` | Reopen |
| `POST /v1/topics/{id}/rescind` | Rescind every open message in the topic, `{ "reason": … }`, without closing it |
| `PUT /v1/messages/{id}/topic` | Move a message to another topic (`{ "topic": topicRef }`), or back to its conversation's default topic (`{ "topic": null }`) |

- **A message names its topic with `topic`:**
  - `{ "id": … }` refers to an existing topic.
  - `{ "key": …, "title": …, "kind": … }` creates the topic on first use and joins it after that. Without `title` the key becomes the title.
  - The inline form never changes an existing topic, and never takes over a conversation's implicit topic (only `POST /v1/topics` with `conversation.id` does).
- **A message sent to a closed topic reopens it,** and the reopen is recorded.
- **MCP tools** (PR 6): `create_topic` (key, title, summary, kind, style, and whether it stands for the current conversation), `close_topic` (key or id, `rescindOpenMessages`), and a `topic` parameter on `notify` and `ask`. **CLI:** `gentbox topic create|close|reopen|list` and `--topic` on `send` and `ask`.

### Rules

A rule may assign a topic when the message names none. Example:

```toml
[[rule]]
name = "One topic per project"
match = { source = "claude-code" }
topic = { key = "project:{project}", title = "{project}", kind = "project" }
```

`{project}` is the conversation's working-directory name. The topic is created on first use, as with the inline form. A rule-assigned topic wins over the conversation's default topic.

### You, in the UI

- **Mute and unmute a topic, implicit ones included:** non-urgent messages in a muted topic go to the inbox only. No popup, toast, native notification, phone push or re-ping, and their waiting senders get `shouldWait: false`, so a hook falls back to the agent's own prompt at once. Urgent messages are still delivered as usual.
- **Mark all read:** acknowledges every open message in the topic. Nothing is answered.
- **Close and reopen, and move a message,** with the same "also withdraw its open messages" option on close.
- Mute is the only way a topic changes how a single message is delivered.

### Phone

- APNs `thread-id` is the message's topic id, implicit topics included, so iOS groups a topic's notifications on the lock screen and in Notification Centre. Notifications already delivered keep their old thread when a takeover or a move happens later.
- **Subtitle:** blocking badge · topic title · conversation title, leaving out the conversation title when it equals the topic title (implicit and `kind: "conversation"` topics).
- The ntfy fallback cannot group by Gentbox topic (it groups by ntfy topic), so the topic title leads the notification text.

### Storage

- **`topics` table:**
  - `key` (nullable, unique when set), `kind`, `implicit`, status, style.
  - The mute flag and the mark-read time.
  - Created-by.
- **Conversations** gain `default_topic_id`.
- **Messages** gain `topic_id` (never null once stored).
- **Retention** never deletes a topic that still has messages. Closed or implicit topics with no messages left are deleted after the message retention period.

## Consequences

- One grouping level in the inbox and one thread per topic on the phone, whatever the agent chose to group by. Agents that never create topics still get one group per conversation.
- The takeover gives an agent a clean way to name and style its own session's group without losing the messages it already sent.
- Global keys can collide between unrelated agents. Keys are free-form so agents can share them on purpose; the docs recommend a prefix such as `project:` or `repo:`.
- Topic listing exposes titles to every local agent; acceptable under the trust model (architecture §12), since message content stays unreadable to the sender token.
- Muting a topic silences blocking prompts in it unless they are urgent. That is what mute means, and `shouldWait: false` keeps them from hanging.

## Alternatives

- **Two levels, topic → conversation** (the first version of this ADR): the user prefers one grouping chosen by the agent.
- **Topics as tags (many per message):** grouping, muting and the lock-screen thread need exactly one home per message.
- **Nested topics:** not needed, and they complicate mute and close.
- **Implicit topics as a UI-only fiction:** mute, mark read and `thread-id` would need special cases; a real hub-managed record keeps every topic alike.
- **Per-agent topic keys:** safer against collisions, but defeats topics shared across agents.

## Amendments

- **2026-10-24: a muted message reports the mute and stays open.** Where this ADR says senders of muted messages get `shouldWait: false` so a hook falls back at once: that is unchanged, but the sender is no longer left to withdraw the message. The ingest response carries the warning `muted` ("Delivered to the inbox without a ping: topic 'X' is muted"), and the message stays open for the user to answer later; the sender may collect the answer with a poll or rescind it itself. Withdrawn means the agent asked (ADR 0007, amendments).
- **2026-10-24: a muted ask is parked.** The hub also parks a muted message that awaits a response (ADR 0007, amendments), so the clean-ups of completion and progress leave it for you to answer later.
