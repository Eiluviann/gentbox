# 0011 Action plans for action_required messages

Status: Accepted, 2026-10-23.

## Context

The user: "Action steps should include an action plan explaining how to complete a given action."

An `action_required` message asks you to do something yourself outside the conversation: sign in, add a key, review a pull request. A title and a summary say what, but not how. The design turns such a message into a "Do it" view with numbered steps you can tick off, with Done, Reply and Defer at the bottom.

## Decision

### `steps`: an ordered plan on the message

```json
"steps": [
  { "text": "Open a terminal in any folder" },
  { "text": "Sign in to npm", "detail": "```bash\nnpm login\n```" },
  { "text": "Check you are signed in as the publishing account", "link": { "url": "https://www.npmjs.com/settings/profile", "label": "npm profile" } }
]
```

| Field | Rule |
| --- | --- |
| `text` | Required. One plain instruction, at most 200 characters. It passes the title guard (ADR 0010): no commands, paths, URLs or raw input |
| `detail` | Optional markdown, at most 2,000 characters. The place for commands (in code blocks), paths and explanations |
| `link` | Optional `{ url, label? }`. `https`, or a scheme on the open allowlist (`claude-cli`, `claude`, `vscode`, `cursor`, `x-apple.systempreferences` for "open System Settings › …"). Opened only on tap, with the URL shown |

- **Size:** 1 to 12 steps.
- **Only for `action_required`:** a message that declares another type and carries `steps` is a 422. A message with `steps` and no declared type defaults to `action_required`, unless a rule sets the type. Steps belong to the action and mean nothing elsewhere.

### Required for `action_required`, enforced by the hub

- **No plan:** in the default `rewrite` mode, the hub adds a single fallback step, "Open the conversation and follow the agent's instructions", linked to the conversation's focus target. The 201 response carries the warning `steps_missing`.
- **Strict mode** (per source, `actionPlan = "reject"`) answers 422 `steps_missing` instead.
- **Why not reject by default:** for the same reason as titles, rejecting a blocking `action_required` message would strand the agent.
- **A step `text` that fails the title guard:** moved into that step's `detail` as a code block and replaced by "Run the command below" (when it starts with a known command) or "Follow the details below". Warning `step_rewritten`.
- **The Done button:** the hub makes sure every `action_required` message offers one. It adds a Done answer when the sender declared none, just as it adds Defer to `deferrable` messages.

### Secrets are redacted everywhere

The title guard gains a **secret scan**. It runs over title, summary, steps (`text` and `detail`) and body, before anything is stored or sent. Matches are replaced by `[redacted]`, with warning `secret_redacted`. The patterns:

- private-key headers;
- GitHub tokens (`ghp_`, `gho_`, `ghs_`, `github_pat_`);
- Anthropic and OpenAI-style keys (`sk-ant-…`, `sk-…` with 20 or more characters);
- ntfy tokens (`tk_…`);
- AWS access key ids (`AKIA…`);
- Slack tokens (`xox?-…`);
- JWTs (`eyJ….eyJ….…`);
- `Bearer <20+ characters>`;
- `password=` / `secret=` / `token=` assignments.

The list is code with table tests, like the guard's.

### Ticking steps

- **Your progress, not an answer.** Ticked steps are stored on the message (`stepsCompleted`, step indices) and shared by the Mac and the iOS app. The iOS app sends ticks as signed envelopes on the ntfy answer topic, like registrations.
- **The agent learns nothing until you tap Done** (or Reply, or Defer). Exposing progress to the agent is Planned.
- **"Do it" is a view, not an action kind.** It opens the steps on the Mac (popover) or in the iOS app.

### Delivery

- **Mac:** popups and the popover show "Do it" as the default button for `action_required`. The steps view lists the steps with their details and links, with Done, Reply and Defer at the bottom.
- **iPhone (APNs):**
  - The payload carries the step texts after the actions and before the body excerpt: up to 5 steps, each trimmed to 100 characters. Details stay out of the payload, and the app loads them in the Content Extension or the app when budget allows.
  - The `action` category offers "Do it" (opens the app's steps view; a foreground action), Done, and Defer on `deferrable` messages.
- **ntfy fallback:** step texts as a numbered list in the notification text (no markdown on iOS), within ntfy's 4,096-byte message limit.

### Producing plans

- **Claude Code adapter (Phase 4), fixed plans for its `action_required` events:**
  - `elicitation_url_dialog`: "Open the link Claude Code shows in the session", "Complete the steps on that page", "Return to the session".
  - `quota_auto_resume_stale`: "Open the session", "Press Return to continue".
  - `StopFailure`: "Open the session", "Check the error message", "Ask Claude to try again".
- **The MCP tools:** `notify` and `ask` require `steps` in their input schema whenever `type` is `action_required`, and their descriptions explain the rule with an example.

## Consequences

- **Every `action_required` message is actionable from the notification alone,** on every surface, with the same plan.
- **Agents must think through the "how",** which also makes their requests clearer.
- **Secret redaction now protects every text field,** not just steps. A false positive redacts a harmless string; the body keeps its context.
- **The APNs payload budget gains a slot** that takes from the body excerpt first.

## Alternatives

- **A markdown "Plan" section inside `body`:** no structure to tick off, nothing to validate, and surfaces would parse markdown to find it.
- **Reject missing plans by default:** strands blocking agents (as with titles).
- **Steps on every type:** decisions and completions have no "how"; keeping steps to `action_required` keeps the UI and the validation simple.

## Amendments

### Amendment (2026-10-24): the MCP tools enforce `steps` in validation, not in the advertised schema

The Claude API rejects `allOf`, `anyOf` and `oneOf` at the root of a tool's input schema (HTTP 400), so the root of `notify` and `ask` stays a plain `{ "type": "object" }`. The decision above is unchanged: `steps` is required whenever `type` is `action_required`. The MCP server enforces it with zod, which answers a call without `steps` with an error at path `steps` ("`steps` is required when `type` is `action_required`"), and the tool and `steps` descriptions state the rule with an example. Status stays Accepted.
