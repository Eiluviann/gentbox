# 0010 Descriptive titles, enforced by the hub

Status: Accepted, 2026-10-23.

## Context

The user: "No commands in titles, should be enforced on architecture level", and "the titles should be descriptive and summarisations".

- **Where titles show:** every surface shows them: popover rows, popups, toasts, native notifications, the phone's lock screen and the Watch. They must read as plain words, short enough for one line.
- **Why commands do harm:** a command, path or raw tool input in a title is unreadable at a glance, leaks detail to the phone and push services, and makes approving something on the lock screen riskier.
- **Why surfaces can't own this:** if each surface had to clean titles up, every client would do it differently, and agents posting straight to the HTTP API would bypass it.

## Decision

### The rule

- **`title`** says in plain words what the agent asks or did. For example "Allow Bash to run the ingest tests?" or "Ingest auth accepts rotated tokens; 12 tests pass", not "Allow Bash: cargo test -p ingest -- --nocapture?".
- **`summary`** follows the same rule. It is the description line of every notification.
- **Commands, paths, URLs and raw tool input belong only in `body`,** which surfaces render as a code box.
- **The rule also covers other titles:** topic, thread and conversation titles, which appear in subtitles and list headers.

### Enforcement: the title guard in the hub

The hub runs the **title guard**, a pure, deterministic function in `gentbox-core`, on every incoming text that a surface shows as a line: message `title` and `summary`, `conversation.title`, and topic and thread titles. No surface or adapter is trusted to have done it. A text **looks like raw input** when any of these holds:

1. it contains a backtick or a code fence;
2. it contains a shell operator: ` | `, `&&`, `||`, `$(`, ` > `, ` >> `, ` < `, `2>`;
3. a word is a command-line flag (`-p`, `--nocapture`: a token starting with `-` or `--` followed by a letter);
4. a word is a path:
   - it starts with `/`, `~`, `./`, `../` or `@scope/`;
   - or it holds two or more `/`;
   - or it holds a `/` and a file extension.

   Bare file names (`lib.rs`), versions (`1.4.0`) and `and/or` pass.
5. a word is an environment assignment (`NAME=value` with an upper-case name);
6. it contains `://` (a URL);
7. it starts with `{` or `[`, or contains `":` (JSON);
8. it starts with a lower-case command name from a fixed list followed by more words. The list: `git`, `npm`, `pnpm`, `npx`, `yarn`, `bun`, `deno`, `node`, `cargo`, `rustc`, `make`, `docker`, `kubectl`, `python`, `python3`, `pip`, `go`, `swift`, `xcodebuild`, `gh`, `curl`, `wget`, `ssh`, `sudo`, `rm`, `mv`, `cp`, `brew`, `tsc`, `vitest`, `pytest`.

The heuristics err towards flagging. A false positive costs a less specific title, never a lost message.

### What happens when the guard flags something

- **Message `title` (default `rewrite` mode):**
  - The original title moves to the top of `body` as a code box headed "Original title".
  - It is replaced by the summary's first sentence, if the summary passes the guard (cut to 120 characters at a word boundary).
  - Otherwise by a fallback phrase built from the message's fields and the conversation's project name:
    - `decision_required`: "Claude Code needs your decision in gentbox".
    - `action_required`: "Action needed in gentbox".
    - `task_completed`: "Claude Code finished a task in gentbox".
    - `generic`: "Update from Claude Code".
- **Message `summary`:** it moves to the body the same way and is dropped, so the notification's description falls back to the title.
- **`conversation.title`:** ignored. Subtitles fall back to the project name.
- **Topic and thread titles:**
  - In an explicit `POST /v1/topics`, `POST /v1/threads` or `PATCH`: 422 `title_looks_like_command`. These calls are not time-critical, so the agent fixes the title.
  - Inline or from a rule template: the key is turned into a title instead ("phase-3-ingest" → "Phase 3 ingest").
- **Telling the sender:**
  - The 201 response carries `warnings: [{ "code": "title_looks_like_command", "field": "title", "message": … }]`. The MCP server hands it to the model in the tool result, so the agent learns to write better titles.
  - The delivery log records every rewrite, which feeds the inbox's "why" line.
- **Strict mode:** per source, `titleGuard = "reject"` in `config.toml` turns rewrites into 422s. This is useful while developing an adapter. The default is `rewrite`, because rejecting a blocking prompt would leave the agent without an answer.

### Producing good titles up front

- **The Claude Code adapter (Phase 4)** builds titles from the hook input. It never copies `tool_input` into a title or summary, and always puts the raw input in the body (the command in a `bash` code box, the path and change for edits, pretty JSON for other tools).
  - `PermissionRequest`:
    - Bash: "Allow Bash to <description>?" from `tool_input.description`, with its first letter lower-cased, when it passes the guard. Otherwise "Allow Bash to run a command in <project>?".
    - Edit, Write, MultiEdit: "Allow editing <file name>?", using the bare file name and never the path.
    - WebFetch: "Allow fetching a page from <host>?".
    - MCP tools: "Allow <Server> to <tool name in words>?", for example `mcp__github__create_issue` → "Allow GitHub to create issue?".
    - Any other tool: "Allow <tool>?".
    - Summary: "<project>: <description>" or "Claude Code is waiting for your approval in <project>".
  - `Stop`: the title is the first sentence of `last_assistant_message`, the summary the next sentences, both through the guard. Fallback "Claude Code finished a task in <project>".
  - `Notification` events: fixed descriptive phrases, never the raw `message`.
- **The MCP tools** (`notify`, `ask`, `create_topic`, `create_thread`) state the rule in their descriptions and their `title` parameter documentation, and return the hub's warnings.
- **The schema** documents the rule on `title` and `summary`.

## Consequences

- Every surface can show `title` and `summary` as-is; no client sanitises.
- Agents that post commands still get through, with a weaker title and a warning. Strict mode exists for adapters under development.
- The heuristic list is code with table tests in PR 1. Changing it is a code change, never a per-surface tweak.
- Raw input still reaches you in the body, which stays on the Mac or travels inside the 4 KB push budget as an excerpt.

## Alternatives

- **Reject by default:** loses or delays blocking prompts whenever an agent slips.
- **Sanitise in each surface:** inconsistent, and agents posting straight to the HTTP API bypass it.
- **A model rewriting titles:** ruled out (no model in the pipeline, ADR 0004).
- **Only documentation:** agents drift, and the lock screen shows whatever they send.
