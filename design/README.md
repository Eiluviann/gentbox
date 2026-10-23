# Gentbox GUI design (Phase 2)

Visual design for every surface, as a Design canvas: <https://claude.ai/artifact/MnphbLb4m2WCubpkVWg9cu> (private; share it from the page's Share menu). `project/` holds the canvas sources; the published canvas also carries `project/sf-symbols.js`, which is generated (see Tools) and kept out of git because it holds Apple's SF Symbols outlines and third-party brand marks.

The canvas reads left to right in three groups: Mac, Settings (inside the main window), iPhone and Watch. Consecutive screens sit in a row, alternative states 120 px below their screen. It follows `docs/architecture.md` and ADRs 0006 to 0009 as of `e906f7e`.

## Foundations

Every message has three fields (ADR 0007): a **type**, a **blocking level** and an **urgent** flag.

| Type | Colour | Symbol | Title phrase |
| --- | --- | --- | --- |
| `decision_required` | Indigo `#5856D6` (text `#4644B8` on `#EFEFFD`) | `arrow.triangle.branch`; `checkmark.shield` when it asks permission (approve and deny) | Your input is needed |
| `action_required` | Teal `#0A8270` (text `#076B5C` on `#E3F4F1`) | `hand.tap` | Action required |
| `task_completed` | Green `#2E9E4F` (text `#146C2F` on `#E7F4EA`) | `checkmark.circle` | Task complete / Goal complete |
| `generic` | Grey `#8E8E93` (text `#636366` on `#F2F2F7`) | `info.circle` | the message title, inbox only |

| Blocking level | Label | Effect |
| --- | --- | --- |
| `blocking` | Agent waiting, red `#D70015`, `pause.circle` | persistent popup, re-pings |
| `deferrable` | Paused · can defer, orange `#E8710A`, `pause.circle` | popup with Defer |
| `non_blocking` | Agent working, grey, `play.circle` | as the type says |

- **Urgent** puts a red `figure.walk.motion` under the type symbol, in the same column, and pulls the item to the top of its list; no background, border or URGENT label. It combines with any type but generic: Time Sensitive on the Mac, iPhone and Watch, through quiet hours and Focus, "URGENT:" before the notification title, and looped ripple rings around the whole popup (red when waiting, orange when paused).
- Lists are one sorted list, urgent first, then waiting, with no category headers; each row shows its blocking label, and the type symbol leads it.
- **Defer** (`forward`) is an answer on deferrable items, on the Mac, the iPhone push and the Watch; the item then reads as Agent working. **Snooze** (`moon.zzz`) is local and never reaches the agent, so it is not a push button.
- **Withdrawn** (`arrow.uturn.backward.circle`): the popup says why and slides out, the inbox row is struck through with the reason in words, the iPhone removes the notification; superseded messages fold into their replacement.
- **Topics and threads** (ADRs 0008 and 0009): a topic is a project or bigger undertaking, or the General category for simple conversations; a thread is a version, a phase or a single conversation. The inbox reads topic → thread → messages: topic header rows (symbol and colour hint, title, menu), thread label rows indented under them (title, collapse, menu); no kind labels, since the symbol already says what a topic is, then message rows; rows carry no conversation label, since topic and thread already place them, and Open still goes to the session. No cards, no counters. Topic and thread menus offer Open, Mark all read, Mute and Close; muting a topic covers its threads; the popover lists only open, unhandled items: closed threads, handled messages and silent updates live in History in the main window. Popups and toasts carry a topic · thread chip; notifications put badge · topic · thread in the subtitle (dropping repeats and "General"), and the iPhone and the Mac group notifications by thread. "Topic" alone always means a Gentbox topic; ntfy's is always called the ntfy topic.
- Actions: Approve `checkmark` (Face ID on the iPhone), Deny `xmark`, Reply `arrowshape.turn.up.left`, Done (the `done` answer on action items: you did it, the agent is told), Open `arrow.up.forward.app`. Deny keeps the system red.
- **Outcomes afterwards**, the same tag on the Mac and the iPhone: "Approved on iPhone" (answered, with where), "Done on Mac" (done), "Deferred · agent working" (still open, now non-blocking), "Marked done, no answer" (dismissed), "Withdrawn · No longer needed" (rescinded, reason in words).
- "URGENT:" follows the urgent flag only; the blocking label is separate, so the boards show an urgent item that does not block (the completed hotfix) and blocking items that are not urgent (the stacked Allow Edit).
- **Titles are descriptive summaries, never commands.** A title says in plain words what the agent asks or did ("Allow Bash to run the ingest tests?", "Ingest auth accepts rotated tokens; 12 tests pass"), short enough for one line in the popover; the command, path or raw input only appears in a code box in the body, never in a title, row or notification line.
- **Action plans.** Every action_required message carries an action plan: numbered steps saying how to do it. Its card and popup lead with **Do it**, which turns the whole popover into the plan; steps can be ticked off locally, and Done, Reply and Defer sit at the bottom (through the 3 s undo).
- Icons are outline SF Symbols by default, never the filled variants.
- Accent `#007AFF` is for controls only. Greys: `#F2F2F7` tracks, 1 px `#E0E0E0` borders, no box inside a box. Type is the system font.
- **Gentbox mark** (menu bar and app icon): a custom symbol, `bubble.left` with a `>_` prompt; red dot and count while agents wait, slash in quiet hours or Focus. An inbox-tray variant was tried for the rename and dropped at the user's request.
- Brand marks stay SVG (Claude, Codex); the ntfy mark is a labelled placeholder.

## Boards

| Group | Board | Variants below it |
| --- | --- | --- |
| Mac | `Main` foundations | |
| | `mac-menubar-icons` | |
| | `mac-popover` next up: the blocked-on message as a full card, the rest as a two-line list, topic and thread as a breadcrumb | `mac-popover-undo` (actioned items held with a bar and Undo before sending), `mac-popover-steps` (Do it opens the action plan), `mac-popover-item-menu` (mute and close from a message's menu), `mac-popover-empty` |
| | `mac-blocking-popup` decision, waiting, urgent | `mac-blocking-stacked`, `mac-blocking-reply`, `mac-blocking-no-waiter`, `mac-popup-deferrable` (action, paused, Defer), `mac-popup-rescinded` (withdrawn transition and rows) |
| | `mac-completion-toast` | |
| | `mac-native-banner` decision, agent working | `mac-native-banner-expanded` |
| | `mac-generic-row` silent inbox rows | |
| | `mac-topic` topic detail: its threads | `mac-thread` (thread detail: open items and a timeline across its sessions) |
| | `mac-conversation` detail: pending action, timeline, markdown body | `mac-outcomes` (answered, done, deferred, dismissed and withdrawn, afterwards in History and the conversation) |
| Settings | `settings-notifications` policy by type, blocking-level and urgent rules, presence | |
| | `settings-quiet-focus` Focus status permission (`INFocusStatusCenter`); Full Disk Access only as a note for builds without the paid team | |
| | `settings-sources` per-source switch, wait-for-answers timeout | |
| | `settings-rules` rules set type, urgent and blocking, saved back to rules.toml | `settings-rules-error` |
| | `settings-phone` delivery (our app or the ntfy app), paired iPhones with Revoke, Apple push key (`.p8`, Key ID, Team ID, masked), ntfy server and token, Summary or Minimal, end-to-end encryption and project icon as big icon (both off), Load full messages over Tailscale (off) | `settings-phone-pairing` |
| iPhone and Watch | `ios-escalation-silent` 1: passive arrival in Notification Centre | |
| | `ios-lock-screen` 2: the Time Sensitive ping 2 min later | `ios-lock-screen-expanded` (long press, approval: Approve, Deny, Reply), `ios-faceid` (Approve asks for Face ID), `ios-lock-screen-choices` (one button per choice, Defer, Reply, Open), `ios-lock-screen-reply` (text reply field), `ios-lock-screen-stacked` (stacked by thread), `ios-lock-screen-undo` (Undo for 3 s after a lock-screen action) |
| | `ios-escalation-handled` 3: replaced by "Approved from phone" or "Handled on Mac" | |
| | `watch` mirrored notifications with buttons; reply by dictation, Scribble or keyboard | |
| | `ios-pairing` scan the Mac's QR code; the ntfy app only as a fallback link | |
| | `ios-inbox` | `ios-inbox-undo` (actioned items held 3 s with Undo) |
| | `ios-detail` | `ios-detail-answered` |
| | `ios-history` | |

## Undo before sending

The same 3 s hold applies on the Mac and the iPhone. Anything acted on (approve, deny, a choice, reply, done, defer, dismiss) shrinks to a small box with a 3 s bar and Undo; nothing is sent while the bar runs, and Undo puts the message back where it was. On the Mac, ⌘Z undoes the last one, closing the popover does not cancel the timer, and several held actions stack, newest on top. On the lock screen, the Content Extension keeps the long-press card open with Undo as its only button; a swipe action replaces the notification with an "Approved" one carrying Undo for the same 3 s.

## Conflicts with the Phase 1 docs

None open. ADR 0007 settled Snooze (local) versus Defer (an answer), permission prompts (decision_required + blocking), the per-source switch and the "wait for answers" timeout (Settings › Sources), and rules edited in the app. Still to confirm there: whether "URGENT:" follows the urgent flag, as drawn.

Open: the undo window. `architecture.md` has the UI send answers straight to the hub actor, which closes the message on the first answer ("first answer wins"). Holding an answer for 3 s needs a decision on where the hold lives (in the app before the command is sent, or as a pending state in the hub), what the other device shows meanwhile, and what happens if it answers during the window (the held answer would then get 409 and show "Already answered on <channel>"). On iOS the hold also has to fit the background time an action response gets.

Settled: descriptive titles (ADR 0010, architecture §5.5). The hub enforces them for message, conversation, topic and thread titles and summaries, so surfaces show `title` and `summary` as they arrive and never sanitise. A flagged title moves to the top of the body as an "Original title" code box and is replaced by the summary's first sentence or a fallback ("Action needed in gentbox"); the inbox why line may mention the rewrite. Claude Code permission titles follow the adapter's patterns: "Allow Bash to <description>?", "Allow editing <file name>?", "Allow fetching a page from <host>?", "Allow <Server> to <tool in words>?"; the boards use these. Also sent: action_required messages must carry an action plan (ordered steps), which the steps view shows; not yet answered.

## Open design questions

1. Is the agent mark (speech bubble with `>_`) right?
2. Keyboard shortcuts on the non-activating panel (⌘↩, ⌘⌫, ⌘R): make the panel key without activating the app, or a global hotkey?
3. Dark-mode boards for popups and windows before Phase 3?
4. Popups say "Action required" without "URGENT:"; should they use the exact notification phrase?
5. The per-project icon override (§10) has no board yet: a Projects list in Settings, or a field in the conversation detail?
6. Should Mac popups and the Watch get the same 3 s undo window?
7. Finished tasks now sit in the popover list until dismissed. Should they clear on their own once read?

## Tools

`tools/` rebuilds the symbol outlines and renders static previews.

- `match.swift` maps SF Symbol names (`names.txt`) to SF Pro private-use codepoints (`pua.txt`) by rendering both and comparing; `map.tsv` is the result, with `minus` corrected by hand to `U+10017D`.
- `extract.py` reads those glyphs from `/Library/Fonts/SF-Pro.ttf` (regular, opsz 17) into a 100-unit frame; `build-symbols.py <map.tsv> <icons.json> project/sf-symbols.js` adds the agent mark and the brand marks (from Zed's agent registry icons). Needs SF Pro and the SF Symbols app installed.
- `preview.py <out-dir> [boards]` fills the icon holes and writes plain HTML for a local look (headless Chrome screenshots).
- Every symbol name was checked against `symbol_restrictions.strings`; the restricted `iphone` and `applewatch` appear only for the iPhone and the Apple Watch.
