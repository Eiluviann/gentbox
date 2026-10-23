# Gentbox GUI design (Phase 2)

Visual design for every surface, as a Design canvas: <https://claude.ai/artifact/MnphbLb4m2WCubpkVWg9cu> (private; share it from the page's Share menu). `project/` holds the canvas sources; the published canvas also carries `project/sf-symbols.js`, which is generated (see Tools) and kept out of git because it holds Apple's SF Symbols outlines and third-party brand marks.

The canvas reads left to right in three groups: Mac, Settings (inside the main window), iPhone and Watch. Consecutive screens sit in a row, alternative states 120 px below their screen. It follows `docs/architecture.md` and ADRs 0006 to 0008 as of `09981d8`.

## Foundations

Every message has three fields (ADR 0007): a **type**, a **blocking level** and an **urgent** flag.

| Type | Colour | Symbol | Title phrase |
| --- | --- | --- | --- |
| `decision_required` | Indigo `#5856D6` (text `#4644B8` on `#EFEFFD`) | `arrow.triangle.branch`; `checkmark.shield` when it asks permission (approve and deny) | Your input is needed |
| `action_required` | Teal `#0A8270` (text `#076B5C` on `#E3F4F1`) | `hand.tap` | Action required |
| `task_completed` | Green `#2E9E4F` (text `#146C2F` on `#E7F4EA`) | `checkmark.circle.fill` | Task complete / Goal complete |
| `generic` | Grey `#8E8E93` (text `#636366` on `#F2F2F7`) | `info.circle.fill` | the message title, inbox only |

| Blocking level | Label | Effect |
| --- | --- | --- |
| `blocking` | Agent waiting, red `#D70015`, `pause.circle.fill` | persistent popup, re-pings |
| `deferrable` | Paused · can defer, orange `#E8710A`, `pause.circle.fill` | popup with Defer |
| `non_blocking` | Agent working, grey, `play.circle.fill` | as the type says |

- **Urgent** is a solid red pill (`clock.badge.exclamationmark`, white on `#D70015`; the waiting label is the soft red tint) that combines with any type but generic: Time Sensitive on the Mac, iPhone and Watch, through quiet hours and Focus, "URGENT:" before the title, and looped ripple rings around the whole popup (red when waiting, orange when paused).
- Lists are one sorted list, waiting first, with no category headers; each row shows its blocking label, and the type symbol leads it.
- **Defer** (`forward`) is an answer on deferrable items, on the Mac, the iPhone push and the Watch; the item then reads as Agent working. **Snooze** (`moon.zzz`) is local and never reaches the agent, so it is not a push button.
- **Withdrawn** (`arrow.uturn.backward.circle`): the popup says why and slides out, the inbox row is struck through with the reason in words, the iPhone removes the notification; superseded messages fold into their replacement.
- **Topics** (ADR 0008): whatever grouping the agent chooses. The inbox is one level, topic then messages; a message without one sits in its conversation's implicit topic, which looks like any topic row. A header row shows the agent's symbol and colour hint (Release 1.4 `shippingbox.fill`, a conversation `bubble.left.and.bubble.right.fill`, Dependency upgrades `wrench.and.screwdriver.fill`), the title, the `kind` as a quiet tag (no counters), and a menu with Open topic, Mark all read, Mute topic and Close topic. Muted topics show a `bell.slash.fill` marker and are listed in Settings › Notifications; closed topics collapse to one `archivebox` row with History. Popups and toasts carry a topic chip; native and iPhone notifications put the topic in the subtitle (blocking label · topic · session), and the iPhone stacks notifications by topic. "Topic" alone always means a Gentbox topic; ntfy's is always called the ntfy topic.
- Actions: Approve `checkmark` (Face ID on the iPhone), Deny `xmark`, Reply `arrowshape.turn.up.left`, Done (the `done` answer on action items: you did it, the agent is told), Open `arrow.up.forward.app`. Deny keeps the system red.
- **Outcomes afterwards**, the same tag on the Mac and the iPhone: "Approved on iPhone" (answered, with where), "Done on Mac" (done), "Deferred · agent working" (still open, now non-blocking), "Marked done, no answer" (dismissed), "Withdrawn · No longer needed" (rescinded, reason in words).
- "URGENT:" follows the urgent flag only; the blocking label is separate, so the boards show an urgent item that does not block (the completed hotfix) and blocking items that are not urgent (the stacked Allow Edit).
- Accent `#007AFF` is for controls only. Greys: `#F2F2F7` tracks, 1 px `#E0E0E0` borders, no box inside a box. Type is the system font.
- **Gentbox mark** (menu bar and app icon): a custom symbol, `bubble.left` with a `>_` prompt; red dot and count while agents wait, slash in quiet hours or Focus. An inbox-tray variant was tried for the rename and dropped at the user's request.
- Brand marks stay SVG (Claude, Codex); the ntfy mark is a labelled placeholder.

## Boards

| Group | Board | Variants below it |
| --- | --- | --- |
| Mac | `Main` foundations | |
| | `mac-menubar-icons` | |
| | `mac-popover` inbox grouped by topic | `mac-popover-topic-menu` (topic menu, muted and closed topics), `mac-popover-empty` |
| | `mac-blocking-popup` decision, waiting, urgent | `mac-blocking-stacked`, `mac-blocking-reply`, `mac-blocking-no-waiter`, `mac-popup-deferrable` (action, paused, Defer), `mac-popup-rescinded` (withdrawn transition and rows) |
| | `mac-completion-toast` | |
| | `mac-native-banner` decision, agent working | `mac-native-banner-expanded` |
| | `mac-generic-row` silent inbox rows | |
| | `mac-topic` topic detail: open items and a timeline across sessions | |
| | `mac-conversation` detail: pending action, timeline, markdown body | `mac-outcomes` (answered, done, deferred, dismissed and withdrawn, afterwards in History and the conversation) |
| Settings | `settings-notifications` policy by type, blocking-level and urgent rules, presence | |
| | `settings-quiet-focus` Focus status permission (`INFocusStatusCenter`); Full Disk Access only as a note for builds without the paid team | |
| | `settings-sources` per-source switch, wait-for-answers timeout | |
| | `settings-rules` rules set type, urgent and blocking, saved back to rules.toml | `settings-rules-error` |
| | `settings-phone` delivery (our app or the ntfy app), paired iPhones with Revoke, Apple push key (`.p8`, Key ID, Team ID, masked), ntfy server and token, Summary or Minimal, end-to-end encryption and project icon as big icon (both off), Load full messages over Tailscale (off) | `settings-phone-pairing` |
| iPhone and Watch | `ios-escalation-silent` 1: passive arrival in Notification Centre | |
| | `ios-lock-screen` 2: the Time Sensitive ping 2 min later | `ios-lock-screen-expanded` (long press, approval: Approve, Deny, Reply), `ios-faceid` (Approve asks for Face ID), `ios-lock-screen-choices` (one button per choice, Defer, Reply, Open), `ios-lock-screen-reply` (text reply field), `ios-lock-screen-stacked` (stacked by topic) |
| | `ios-escalation-handled` 3: replaced by "Approved from phone" or "Handled on Mac" | |
| | `watch` mirrored notifications with buttons; reply by dictation, Scribble or keyboard | |
| | `ios-pairing` scan the Mac's QR code; the ntfy app only as a fallback link | |
| | `ios-inbox` | |
| | `ios-detail` | `ios-detail-answered` |
| | `ios-history` | |

## Conflicts with the Phase 1 docs

None open. ADR 0007 settled Snooze (local) versus Defer (an answer), permission prompts (decision_required + blocking), the per-source switch and the "wait for answers" timeout (Settings › Sources), and rules edited in the app. Still to confirm there: whether "URGENT:" follows the urgent flag, as drawn.

## Open design questions

1. Is the agent mark (speech bubble with `>_`) right?
2. Inline Approve/Deny in popover rows and one-click Approve on collapsed stacked panels: keep, or require the full panel?
3. Keyboard shortcuts on the non-activating panel (⌘↩, ⌘⌫, ⌘R): make the panel key without activating the app, or a global hotkey?
4. Dark-mode boards for popups and windows before Phase 3?
5. Popups say "Action required" without "URGENT:"; should they use the exact notification phrase?
6. The per-project icon override (§10) has no board yet: a Projects list in Settings, or a field in the conversation detail?

## Tools

`tools/` rebuilds the symbol outlines and renders static previews.

- `match.swift` maps SF Symbol names (`names.txt`) to SF Pro private-use codepoints (`pua.txt`) by rendering both and comparing; `map.tsv` is the result, with `minus` corrected by hand to `U+10017D`.
- `extract.py` reads those glyphs from `/Library/Fonts/SF-Pro.ttf` (regular, opsz 17) into a 100-unit frame; `build-symbols.py <map.tsv> <icons.json> project/sf-symbols.js` adds the agent mark and the brand marks (from Zed's agent registry icons). Needs SF Pro and the SF Symbols app installed.
- `preview.py <out-dir> [boards]` fills the icon holes and writes plain HTML for a local look (headless Chrome screenshots).
- Every symbol name was checked against `symbol_restrictions.strings`; the restricted `iphone` and `applewatch` appear only for the iPhone and the Apple Watch.
