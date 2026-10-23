# 0008 Topics

Status: Accepted, 2026-10-23.

## Context

The user: "The model should also be able to create topics and associating messages with them to group notifications". A conversation is one agent session. Work often spans several sessions and agents: "Release 1.4" touches an API session and a web session. Its messages should read as one group in the inbox and on the lock screen, and be muted or cleared together.

**Three things are called "topic" in this project. The docs keep them apart:**

- **Topic** (or Gentbox topic): this ADR's grouping.
- **ntfy topic:** ntfy's channel name. The docs always say "ntfy topic", for example the ntfy answer topic.
- **`apns-topic`:** the APNs request header that carries the app's bundle id.

## Decision

### Model

- **A topic is created by an agent.** Fields:
  - `id` (hub-assigned ULID).
  - `key`, stable and chosen by the agent, unique per hub across all agents.
  - `title`, optional `summary`, and an optional `style` (SF Symbol name and colour hint, shown only when the symbol is on the app's allowed list).
  - `status` (`open` or `closed`), `createdBy` (agent and conversation), and the `createdAt`, `updatedAt` and `closedAt` timestamps.
- **Flat.** Topics are not nested.
- **Distinct from the conversation.** A message always belongs to its conversation and optionally to one topic. One topic may span many conversations and agents, and one conversation may send messages into several topics.
- **The topic only groups.** A message keeps its own `type`, `urgent` and `blocking`. Coalescing, supersede, fold and the implicit rescinds stay per conversation (architecture §7).

### API (sender token)

| Method and path | Purpose |
| --- | --- |
| `POST /v1/topics` | Upsert by `key` (`topicCreate`): 201 with the new topic, or 200 with the existing one unchanged |
| `GET /v1/topics?status=open` | List topics: id, key, title, status, open-message count. Topics only, never their messages: agents need to find a topic to join it, but the sender token still cannot read messages (architecture §8) |
| `GET /v1/topics/{id}` | One topic (`topicRecord`) |
| `PATCH /v1/topics/{id}` | Rename, change the summary or style (`topicUpdate`) |
| `POST /v1/topics/{id}/close` | Close (`topicClose`); open messages stay unless `rescindOpenMessages: true` |
| `POST /v1/topics/{id}/reopen` | Reopen |
| `POST /v1/topics/{id}/rescind` | Rescind every open message in the topic, `{ "reason": … }`, without closing it |
| `PUT /v1/messages/{id}/topic` | Move a message to another topic (`{ "topic": topicRef }`) or out of any (`{ "topic": null }`) |

- **A message names its topic with `topic`:**
  - `{ "id": … }` refers to an existing topic.
  - `{ "key": …, "title": … }` creates the topic on first use and joins it after that. Without `title` the key becomes the title.
  - The inline form never changes an existing topic.
- **A message sent to a closed topic reopens it,** and the reopen is recorded.
- **MCP tools** (PR 7): `create_topic` (key, title, summary, style), `close_topic` (key or id, `rescindOpenMessages`), and a `topic` parameter on `notify` and `ask`. **CLI:** `gentbox topic create|close|reopen|list` and `--topic` on `send` and `ask`.

### Rules

A rule may assign a topic when the message has none. Example:

```toml
[[rule]]
name = "One topic per repository"
match = { source = "claude-code" }
topic = { key = "repo:{project}", title = "{project}" }
```

`{project}` is the conversation's working-directory name. The topic is created on first use, as with the inline form.

### You, in the UI

- **Mute and unmute a topic:** non-urgent messages in a muted topic go to the inbox only. No popup, toast, native notification, phone push or re-ping, and their waiting senders get `shouldWait: false`, so a hook falls back to the agent's own prompt at once. Urgent messages are still delivered as usual: muting a topic never hides something its sender marked urgent.
- **Mark all read:** acknowledges every open message in the topic. Nothing is answered.
- **Close and reopen** from the UI too, with the same "also withdraw its open messages" option.
- Mute is the only way a topic changes how a single message is delivered.

### Phone

- APNs `thread-id` is the topic id, falling back to the conversation id, so iOS groups a topic's notifications on the lock screen and in Notification Centre.
- The topic title appears in the notification subtitle in place of the project: badge, then topic, then conversation.
- The ntfy fallback cannot group by Gentbox topic (it groups by ntfy topic), so it only puts the topic title in the subtitle text.

### Storage

- A `topics` table (key unique).
- A nullable `topic_id` on messages.
- The mute flag and the mark-read time on the topic.
- Retention never deletes a topic that still has messages. Closed topics with no messages left are deleted after the message retention period.

## Consequences

- **The inbox gains a level:** topic, then conversation, then message. Messages without a topic group by conversation as before.
- **Global keys can collide between unrelated agents** (two "release" topics). Keys are free-form so agents can share them on purpose; the docs recommend a prefix such as `repo:` or the project name.
- **Topic listing exposes titles to every local agent.** Acceptable under the trust model (architecture §12): titles are chosen by agents, and message content stays unreadable to the sender token.
- **Muting a topic silences blocking prompts in it unless they are urgent.** That is what mute means; `shouldWait: false` keeps such prompts from hanging.

## Alternatives

- **Topics as tags (many per message):** more flexible, but grouping, muting and the lock-screen thread need exactly one home per message.
- **Nested topics:** not needed for the grouping asked for, and they complicate mute and close.
- **Reuse the conversation as the group:** cannot span sessions or agents, which is the point of the request.
- **Per-agent topic keys:** safer against collisions but defeats cross-agent topics.
