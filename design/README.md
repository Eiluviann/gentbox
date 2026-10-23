# Agent Notifier GUI design (Phase 2)

Visual design for every surface, as a Design canvas: <https://claude.ai/artifact/MnphbLb4m2WCubpkVWg9cu> (private; share it from the page's Share menu). `project/` holds the canvas sources; the published canvas also carries `project/sf-symbols.js`, which is generated (see Tools) and kept out of git because it holds Apple's SF Symbols outlines and third-party brand marks.

The canvas reads left to right in three groups: Mac, Settings (inside the main window), iPhone and Watch. Consecutive screens sit in a row, alternative states 120 px below their screen. It follows `docs/architecture.md` as of `d4a4b61`.

## Foundations

| Class | UI name | Colour | Symbol |
| --- | --- | --- | --- |
| `action_blocking` | Blocked | Red `#D70015` (text `#A8000F` on `#FDECEE`) | `exclamationmark.bubble.fill` |
| `action_nonblocking` | Question | Orange `#E8710A` (text `#A64B00` on `#FFF2E6`) | `exclamationmark.bubble.fill` |
| `completion` | Finished | Green `#2E9E4F` (text `#146C2F` on `#E7F4EA`) | `checkmark.circle.fill` |
| `generic` | Other | Grey `#8E8E93` (text `#636366` on `#F2F2F7`) | `info.circle.fill` |

- Action required has one symbol; blocking and non-blocking differ by colour, red and orange, which also differ in lightness.
- **Permission prompts** (messages with approve or deny actions, the ones titled "Action required") swap the class symbol for `hand.raised.fill`, in their class colour. The hand appears nowhere else.
- Persistent popups ripple: looped rings in the class colour behind the symbol, one still ring under Reduce Motion. Toasts, banners and rows never ripple.
- Category headers in lists are plain text; the symbols live on the rows.
- Deny keeps the system red for destructive actions. Every class has its own glyph, so colour is never the only cue.
- Accent `#007AFF` is for controls only. Greys: `#F2F2F7` tracks, 1 px `#E0E0E0` borders, no box inside a box.
- Actions: Approve `checkmark`, Deny `xmark`, Reply `arrowshape.turn.up.left`, Open conversation `arrow.up.forward.app`, Snooze `moon.zzz` (the agent keeps working meanwhile), More `ellipsis`.
- **Agent mark** (menu bar and app icon): a custom symbol, `bubble.left` with a `>_` prompt. States: plain (idle), dot (unread), red dot plus count (blocked), slash (quiet hours or Focus). Phase 3 rebuilds it as a real custom symbol in the SF Symbols app.
- **Notification copy** (architecture §10), on Mac native notifications, the lock screen and the Watch: the title is the fixed phrase (URGENT: Action required, URGENT: Your input is needed, Action required, Your input is needed, Task complete, Goal complete), the text is the summary, the thumbnail is the project icon or else the agent icon. On the Mac the subtitle carries the question and the project. Popups use the same phrases without "URGENT:", since the red ripple carries it.
- Type is the system font (SF Pro): 13 px body, 11 px floor, 22 px title on the Mac; 17 px body on iOS. No design system is attached: the listed ones carry other products' brands.
- Brand marks stay SVG (Claude, Codex from Zed's agent registry icons); the ntfy mark is a labelled placeholder.

## Boards

| Group | Board | Variants below it |
| --- | --- | --- |
| Mac | `Main` foundations | |
| | `mac-menubar-icons` | |
| | `mac-popover` inbox, blocked first, one row per conversation | `mac-popover-empty` |
| | `mac-blocking-popup` persistent non-activating panel | `mac-blocking-stacked`, `mac-blocking-reply`, `mac-blocking-no-waiter` |
| | `mac-completion-toast` | |
| | `mac-native-banner` non-blocking | `mac-native-banner-expanded` |
| | `mac-generic-row` silent inbox rows | |
| | `mac-conversation` detail: pending action, timeline, markdown body | |
| Settings | `settings-notifications` per-class matrix, silent-then-ping, presence | |
| | `settings-quiet-focus` automatic Focus detection (Full Disk Access), Shortcuts fallback | |
| | `settings-sources` | |
| | `settings-rules` | `settings-rules-error` |
| | `settings-phone` ntfy.sh or self-hosted, subscribe QR, topics, summary or minimal | `settings-phone-pairing` (optional iPhone app) |
| iPhone and Watch | `ios-lock-screen` | `ios-lock-screen-expanded` (summary vs minimal) |
| | `watch` | |
| | `ios-inbox`, `ios-detail` (optional app, Tailscale) | `ios-detail-answered` |

## Conflicts with the Phase 1 docs

1. **Snooze tells the agent to keep working** (the user's decision during Phase 2). §5.2 and the schema still make `snooze` a non-answer never returned to a waiting sender; this needs a returned status (for example `snoozed` with `snoozedUntil`) and a Phase 4 mapping for Claude Code.
2. **Permission as its own kind.** The hand marks messages with approve or deny actions, which matches the title rule in §10. If permission should become a fifth class or a schema field, that is a schema change; asked in the canvas comments.
3. **Per-source on/off switch** (Sources): not in the architecture, where per-source policy is Planned.
4. **Rules edited in the UI** write `rules.toml`, and the **Try a message** tester classifies without ingesting: both are additions for PR 1/3.
5. **Wait for answers from Agent Notifier** (Sources › Claude Code) is a plugin setting with no home in `config.toml` yet.

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
