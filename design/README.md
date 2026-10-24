# Gentbox GUI design (Phase 2)

Visual design for every surface, as a Design canvas: <https://claude.ai/artifact/MnphbLb4m2WCubpkVWg9cu> (private; share it from the page's Share menu). `project/` holds the canvas sources; the published canvas also carries `project/sf-symbols.js`, which is generated (see Tools) and kept out of git because it holds Apple's SF Symbols outlines.

**Scope: version 1 is Mac only** (the user's decision, 2026-10-24). The canvas reads left to right in groups: Foundations and menu bar, Popover, Popups, Toast and notifications, Main window, Settings (inside the main window), and **Later · mobile release** (iPhone and Watch), which is the design for the mobile release and not part of v1. In every v1 group the light boards form the top row and their dark twins the row below, each twin straight under its light board; the mobile group stays light. Every v1 flow (answer, Defer, Done, Snooze, Dismiss, rescind, re-pings through Repeat sound) works on the Mac alone; v1 boards show no phone rows, and the iPhone pane in Settings is tagged Planned. It follows `docs/architecture.md` and ADRs 0006 to 0011.

## Foundations

Every message has three fields (ADR 0007): a **type**, a **blocking level** and an **urgent** flag.

| Type | Colour, light / dark | Text on tint, light / dark | Symbol | Title phrase |
| --- | --- | --- | --- | --- |
| `decision_required` | Indigo `#5856D6` / `#7D7AFF` | `#4644B8` on `#EFEFFD` / `#BDBCFF` on `#2F2E55` | `arrow.triangle.branch`; `checkmark.shield` when it asks permission (approve and deny) | Your input is needed |
| `action_required` | Teal `#0A8270` / `#2EC5AC` | `#076B5C` on `#E3F4F1` / `#7FE3D2` on `#163B35` | `hand.tap` | Action required |
| `task_completed` | Green `#2E9E4F` / `#30D158` | `#146C2F` on `#E7F4EA` / `#84E3A0` on `#1A3A24` | `checkmark.circle` | Task complete / Goal complete |
| `generic` | Grey `#8E8E93` / `#98989D` | `#636366` on `#F2F2F7` / `#C7C7CC` on `#3A3A3C` | `info.circle` | the message title, inbox only |

| Blocking level | Label | Colour, light / dark | Effect |
| --- | --- | --- | --- |
| `blocking` | Agent waiting, `pause.circle` | red `#D70015` / `#FF453A`, text `#A8000F` on `#FDECEE` / `#FFA59F` on `#4A1E1D` | persistent popup; sound repeats as set (once, N times or until handled, every N s) |
| `deferrable` | Paused · can defer, `pause.circle` | orange `#E8710A` / `#FF9F0A`, text `#A64B00` on `#FFF2E6` / `#FFC472` on `#47300F` | popup with Defer |
| `non_blocking` | Agent working, `play.circle` | grey | as the type says |

- **Urgent** puts a red `figure.walk.motion` under the type symbol, in the same column, and pulls the item to the top of its list; no background, border, label or "URGENT:" title prefix anywhere (the user's choice, badge only). It stays red in dark mode (`#FF453A`). Native Mac banners have no badge slot, so an urgent one leads its subtitle with "Urgent ·" before the blocking badge. It combines with any type but generic: Time Sensitive on the Mac (and later the iPhone and Watch), through quiet hours and Focus, and looped ripple rings around the whole popup (red when waiting, orange when paused).
- **Popover (an accordion):** the whole popover is one white surface (`#2C2C2E` in dark), with a rule between items and no grey list. One item is open as the full card, by default next up (the message an agent is blocked on): a row's order (type symbol, title, breadcrumb, state) with its body and buttons below. Every other item is a two-line row. Clicking a row opens it in place and folds the open one back into its row; the order never changes, and closing the popover returns to next up (`mac-popover`, `mac-popover-accordion`). Line two is the breadcrumb: topic › thread · agent mark · time ago, with faded dots. Only paused (orange) and waiting (red) show a state icon, a centred `pause.circle` on the right. No section titles, no counters.
- **Defer** (`forward`) is an answer on deferrable items (on the Mac in v1; the iPhone push and the Watch later); the item then reads as Agent working. **Snooze** (`moon.zzz`) is local and never reaches the agent, so it is not a push button.
- **Dismiss** (`xmark.circle`) clears an item without answering it. Dismissal is never a refusal: nothing is denied, and an agent still waiting can be answered in its session. The no-waiter popup calls it **Clear**.
- **Withdrawn** (`arrow.uturn.backward.circle`) appears only when the agent explicitly withdraws a message: the popup says why and slides out, the inbox row is struck through with the reason in words, and a native notification for it is removed. Superseded messages fold into their replacement. When the hub clears a message because the agent reported progress or finished (`rescinded` with reason `handled_in_session`), it reads **Handled in the session** (`terminal`), is not struck through and never says Withdrawn.
- **Topics and threads** (ADRs 0008 and 0009): a topic is a project or bigger undertaking, or the General category for simple conversations; a thread is a version, a phase or a single conversation. In the popover they appear only as the breadcrumb; Mute, Move to topic and Close sit in each message's menu (`mac-popover-item-menu`), and the main window lists topics with their threads in the sidebar (`mac-topic`, `mac-thread`). Muting a topic covers its threads. **A topic or thread with open items can't be closed:** Close shows how many are open ("Close thread v1.1… 3 open") and leads to them, and stays off until none are left (`mac-popover-dismiss`). **Select to dismiss** is a separate multi-select action for clearing several items at once. **The General topic can't be closed** and has no Close. The popover lists only open, unhandled items; handled messages, closed threads and silent updates live in History. Popups and toasts carry a topic · thread chip. "Topic" alone always means a Gentbox topic.
- Actions: Approve `checkmark` (Face ID on the iPhone, later), Deny `xmark`, Reply `arrowshape.turn.up.left`, Done (the `done` answer on action items: you did it, the agent is told), Dismiss `xmark.circle`, Open `arrow.up.forward.app`. Deny keeps the system red, the increased-contrast `#D70015` in light and `#FF453A` for its symbol in dark, with a lighter `#FF7F77` label so it passes on a dark button.
- **Outcomes afterwards**, the same tag wherever the message is listed (`mac-outcomes`): "Approved on Mac" or "Answered in the terminal" (answered, with where), "Done on Mac" (done), "Deferred · agent working" (still open, now non-blocking), "Dismissed on Mac" (dismissed: cleared without an answer, never a refusal), "Handled in the session" (the hub cleared it after progress or completion), "Withdrawn · No longer needed" (explicitly withdrawn by the agent, its reason in words).
- Urgent and blocking are separate: the boards show an urgent item that does not block (the completed hotfix) and blocking items that are not urgent (the stacked Allow editing cart.ts).
- **Titles are descriptive summaries, never commands.** A title says in plain words what the agent asks or did ("Allow Bash to run the ingest tests?", "Ingest auth accepts rotated tokens; 12 tests pass"), short enough for one line in the popover; the command, path or raw input only appears in a code box in the body, never in a title, row or notification line.
- **Action plans.** Every action_required message carries an action plan: numbered steps saying how to do it. Its card and popup offer **Do it**, Done and Defer; Do it turns the whole popover into the plan; steps can be ticked off (stored with the message, shared with the iPhone later, never sent to the agent), and Done, Reply and Defer sit at the bottom (through the 3 s undo). Settled in ADR 0011 (§5.6): 1 to 12 steps, each a plain instruction with optional markdown detail and a link opened only on tap; the hub supplies a one-step plan when the agent sends none.
- Icons are outline SF Symbols by default, never the filled variants (the one exception is `checkmark.circle.fill` for a selected item in Select to dismiss, the system's own selection mark).
- Accent `#007AFF` / `#0A84FF` is for controls only; a primary button fills with the slightly deeper `#0070EB` / `#0A6FDF` so its white label passes 4.5:1 (see Open design questions). Greys: `#F2F2F7` tracks, 1 px `#E0E0E0` borders, no box inside a box. Type is the system font.
- **Gentbox mark** (menu bar and app icon): a custom symbol, `bubble.left` with a `>_` prompt; red dot and count while agents wait, slash in quiet hours or Focus. An inbox-tray variant was tried for the rename and dropped at the user's request.
- **Agent marks** are neutral placeholders on the boards (a rounded square for Claude Code, a circle for Codex); the app ships the real Claude and Codex marks as icons. No brand logos appear in the mockups.

## Dark mode

Every colour token in `project/an.css` holds both values as `light-dark(light, dark)`. A page follows `prefers-color-scheme` (`:root { color-scheme: light dark }`), and `data-theme="light"` or `data-theme="dark"` on any element pins it and everything inside it, so the canvas shows both side by side. Every board root carries its `data-theme`, so a viewer in dark mode still sees the light boards light. The dark twins (`*-dark.dc.html`) are generated: each is its light board with `data-theme="dark"`, and its links point at other dark twins. Edit the light board and rerun `tools/make-dark.py`.

The dark palette follows macOS conventions:

- **Raised surfaces get lighter.** Window content `#1E1E1E` (the system's `controlBackgroundColor`), sidebar `#28282A`, settings tiles `#262628`, popover, popups and toasts `#2C2C2E`, menus `#353537`. The desktop behind them is `#1B2028`.
- **System-like separators.** Rules are white at 12 % (faint rules 7 %, control borders 16 %), like `separatorColor`. Popovers, popups and menus get a white 14 % hairline and a deeper shadow instead of the light-mode grey border.
- **Colours lift for dark.** Type and state colours take Apple's dark values (system red `#FF453A`, orange `#FF9F0A`, green `#30D158`, grey `#98989D`, the increased-contrast indigo `#7D7AFF`, teal raised to `#2EC5AC`). Tags put a light ink on a deep tint of the same hue.
- **Text keeps AA.** Ink `#F5F5F7`, secondary `#C7C7CC`, captions `#AEAEB2` (lifted from `#98989D` so 11 px captions pass on a grey tag).

### Contrast

`tools/contrast.py` checks every text and icon pair in `an.css`, laying translucent fills over the surface they sit on; it exits non-zero when a pair fails. Every pair passes in both modes: text at 4.5:1 or more (lowest: Deny label on a dark button 4.62, white on a primary button 4.65 light), icons and other marks at 3:1 or more (lowest: paused orange on white 3.09). Rerun with `--markdown` to refresh this table.

<!-- contrast:start -->
| Pair | Where | Minimum | Light | Dark |
| --- | --- | --- | --- | --- |
| `ink` on `surface` | Body text in the main window | 4.5:1 | 16.83 | 15.31 |
| `ink` on `elevated` | Body text in the popover, popups and toasts | 4.5:1 | 16.83 | 12.80 |
| `ink` on `sidebar` | Sidebar items | 4.5:1 | 15.46 | 13.51 |
| `ink` on `card` | Settings tiles | 4.5:1 | 16.83 | 13.87 |
| `ink` on `menu` | Menu items | 4.5:1 | 16.14 | 11.27 |
| `ink` on `banner` | Native banner text | 4.5:1 | 15.45 | 12.55 |
| `ink` on `control` | Button labels | 4.5:1 | 16.83 | 10.42 |
| `ink` on `field` | Text fields | 4.5:1 | 16.83 | 15.31 |
| `ink` on `seg-on` | Selected segment | 4.5:1 | 16.83 | 5.50 |
| `ink` on `track` | Code boxes and grey tags | 4.5:1 | 15.08 | 9.97 |
| `ink-2` on `surface` | Secondary text in the window | 4.5:1 | 9.12 | 9.90 |
| `ink-2` on `elevated` | Secondary text in the popover | 4.5:1 | 9.12 | 8.28 |
| `ink-2` on `card` | Secondary text on tiles | 4.5:1 | 9.12 | 8.97 |
| `ink-2` on `track` | Tag text on a track | 4.5:1 | 8.18 | 6.45 |
| `ink-2` on `wait-soft` | Body text on a red callout | 4.5:1 | 8.00 | 8.34 |
| `ink-2` on `warn-soft` | Body text on a warning callout | 4.5:1 | 8.37 | 7.86 |
| `ink-3` on `surface` | Captions and breadcrumbs in the window | 4.5:1 | 5.99 | 7.54 |
| `ink-3` on `elevated` | Captions and breadcrumbs in the popover | 4.5:1 | 5.99 | 6.30 |
| `ink-3` on `sidebar` | Sidebar headings | 4.5:1 | 5.50 | 6.65 |
| `ink-3` on `card` | Captions on tiles | 4.5:1 | 5.99 | 6.83 |
| `ink-3` on `track` | Agent working and Withdrawn tags | 4.5:1 | 5.37 | 4.91 |
| `ink-3` on `track` | Grey tags in the window | 4.5:1 | 5.37 | 5.97 |
| `ink-3` on `menu` | Disabled menu items | 4.5:1 | 5.74 | 5.55 |
| `accent-ink` on `surface` | Links in the window | 4.5:1 | 5.80 | 5.89 |
| `accent-ink` on `elevated` | Links in the popover | 4.5:1 | 5.80 | 4.92 |
| `on-fill` on `accent-fill` | Primary button, selected menu item, chips | 4.5:1 | 4.65 | 4.83 |
| `danger-ink` on `control` | Deny button label | 4.5:1 | 5.38 | 4.62 |
| `on-fill` on `badge` | Count badge | 4.5:1 | 5.38 | 4.78 |
| `dec-ink` on `dec-soft` | Decision tag | 4.5:1 | 6.63 | 7.18 |
| `act-ink` on `act-soft` | Action tag | 4.5:1 | 5.66 | 8.08 |
| `done-ink` on `done-soft` | Completed tag | 4.5:1 | 5.76 | 8.05 |
| `info-ink` on `info-soft` | Generic tag | 4.5:1 | 5.37 | 6.74 |
| `wait-ink` on `wait-soft` | Agent waiting tag | 4.5:1 | 6.89 | 7.45 |
| `paused-ink` on `paused-soft` | Paused · can defer tag | 4.5:1 | 5.26 | 7.89 |
| `wait-ink` on `elevated` | Urgent and waiting words on the popover | 4.5:1 | 7.86 | 7.39 |
| `wait-ink` on `surface` | Urgent and waiting words in the window | 4.5:1 | 7.86 | 8.84 |
| `paused-ink` on `elevated` | Paused words on the popover | 4.5:1 | 5.79 | 8.89 |
| `done-ink` on `elevated` | Done words on the popover | 4.5:1 | 6.53 | 8.94 |
| `act-ink` on `surface` | Done on Mac in the window | 4.5:1 | 6.43 | 10.97 |
| `menubar-ink` on `menubar` | Menu-bar clock | 4.5:1 | 18.83 | 16.61 |
| `dec` on `elevated` | Decision symbol | 3:1 | 5.65 | 4.05 |
| `act` on `elevated` | Action symbol | 3:1 | 4.73 | 6.44 |
| `done` on `elevated` | Completed symbol | 3:1 | 3.43 | 6.89 |
| `info` on `elevated` | Generic symbol | 3:1 | 3.26 | 4.85 |
| `wait` on `elevated` | Waiting pause and urgent figure | 3:1 | 5.38 | 4.09 |
| `paused` on `elevated` | Paused pause symbol | 3:1 | 3.09 | 6.78 |
| `dec` on `surface` | Decision symbol in the window | 3:1 | 5.65 | 4.84 |
| `act` on `surface` | Action symbol in the window | 3:1 | 4.73 | 7.70 |
| `done` on `surface` | Completed symbol in the window | 3:1 | 3.43 | 8.25 |
| `wait` on `surface` | Waiting symbol in the window | 3:1 | 5.38 | 4.89 |
| `paused` on `surface` | Paused symbol in the window | 3:1 | 3.09 | 8.11 |
| `accent` on `sidebar` | Sidebar symbols | 3:1 | 3.69 | 4.03 |
| `accent` on `elevated` | Toggle on, progress bar | 3:1 | 4.02 | 3.82 |
| `danger` on `control` | Deny symbol | 3:1 | 5.38 | 3.33 |
| `topic-release` on `elevated` | Topic symbol | 3:1 | 3.50 | 6.38 |
| `topic-deps` on `sidebar` | Topic symbol in the sidebar | 3:1 | 4.79 | 5.73 |
| `warn-ink` on `warn-soft` | Warning symbol | 3:1 | 5.44 | 8.44 |
| `wait` on `menubar` | Menu-bar red dot | 3:1 | 4.83 | 4.87 |
<!-- contrast:end -->

### Hand-off to `Tokens.swift`

Suggested names for `apps/apple/Gentbox/Tokens.swift`. "System" means a dynamic system colour that matches the design closely enough to use as is; "Pair" means an explicit light/dark pair, for example through `NSColor(name:dynamicProvider:)` choosing on `appearance.bestMatch(from: [.aqua, .darkAqua])`. Tokens marked "none" are drawn by the system (buttons, menus, banners, the menu bar) or exist only on the canvas.

| `an.css` | Light | Dark | Swift token | Source |
| --- | --- | --- | --- | --- |
| `--ink` | `#1D1D1F` | `#F5F5F7` | `Tokens.ink` | System: `NSColor.labelColor` |
| `--ink-2` | `#48484A` | `#C7C7CC` | `Tokens.secondaryInk` | Pair (`secondaryLabelColor` is lighter in light mode) |
| `--ink-3` | `#636366` | `#AEAEB2` | `Tokens.tertiaryInk` | Pair (`secondaryLabelColor` on white is 3.9:1, under AA for 11 px captions) |
| `--on-fill` | `#FFFFFF` | `#FFFFFF` | `Tokens.onFill` | `Color.white` |
| `--surface` | `#FFFFFF` | `#1E1E1E` | `Tokens.surface` | System: `NSColor.controlBackgroundColor` |
| `--sidebar` | `#F5F5F7` | `#28282A` | none | System sidebar material (`NavigationSplitView`) |
| `--card` | `#FFFFFF` | `#262628` | `Tokens.card` | Pair, or the grouped `Form` section background |
| `--elevated` | `#FFFFFF` | `#2C2C2E` | `Tokens.popoverBackground` | Pair: the popover is white, not the default popover material; also popups and toasts |
| `--menu` | white 98 % | `#353537` 98 % | none | System menus |
| `--banner` | `#F6F6F8` 96 % | `#2E2E30` 96 % | none | System notification banners |
| `--control` | `#FFFFFF` | `#3A3A3C` | none | System bordered buttons and pop-up buttons |
| `--field` | `#FFFFFF` | `#1E1E1E` | `Tokens.field` | System: `NSColor.textBackgroundColor` |
| `--seg-on` | `#FFFFFF` | `#636366` | none | System segmented control |
| `--chip` | `#FFFFFF` | white 4 % | `Tokens.chip` | Pair (topic · thread chip) |
| `--track` | `#F2F2F7` | white 8 % | `Tokens.track` | Pair (code boxes, grey tags, progress tracks) |
| `--hover` | black 3.5 % | white 5 % | `Tokens.hover` | Pair |
| `--sel` | black 7 % | white 10 % | `Tokens.selection` | System: `unemphasizedSelectedContentBackgroundColor` |
| `--accent-soft` | `#007AFF` 7 % | `#0A84FF` 18 % | `Tokens.accentSoft` | `Color.accentColor.opacity(0.07 / 0.18)` (selected rows in Select to dismiss) |
| `--toggle-off` | `#E0E0E5` | `#48484A` | none | System switch |
| `--scrim` | black 18 % | black 50 % | none | System sheets |
| `--code-on-tint` | white 70 % | black 25 % | `Tokens.codeOnTint` | Pair (a code box on a warning callout) |
| `--line` | `#E0E0E0` | white 12 % | `Tokens.separator` | System: `NSColor.separatorColor` |
| `--line-faint` | `#ECECEE` | white 7 % | `Tokens.faintSeparator` | Pair |
| `--control-line` | `#D1D1D6` | white 16 % | `Tokens.controlLine` | Pair (custom bordered controls only) |
| `--panel-line` | `#E0E0E0` | white 14 % | `Tokens.panelBorder` | Pair (popover, popup and toast hairline) |
| `--dot` | `#C7C7CC` | `#636366` | `Tokens.dot` | Pair (breadcrumb dots, empty-state symbols) |
| `--strike` | black 25 % | white 30 % | `Tokens.strike` | Pair (Withdrawn strike-through) |
| `--accent` | `#007AFF` | `#0A84FF` | `Tokens.accent` | System: `Color.accentColor` / `controlAccentColor` |
| `--accent-ink` | `#0062CC` | `#409CFF` | `Tokens.link` | System: `NSColor.linkColor` |
| `--accent-fill` | `#0070EB` | `#0A6FDF` | `Tokens.accentFill` | Pair, or the system prominent button (Open design questions) |
| `--focus-ring` | `#007AFF` 22 % | `#0A84FF` 40 % | none | System: `keyboardFocusIndicatorColor` |
| `--danger` | `#D70015` | `#FF453A` | `Tokens.danger` | Pair (`systemRed` is `#FF3B30` in light; the design keeps the increased-contrast red) |
| `--danger-ink` | `#D70015` | `#FF7F77` | `Tokens.dangerInk` | Pair (Deny label) |
| `--badge` | `#D70015` | `#E0201A` | `Tokens.badge` | Pair (white count on it) |
| `--badge-grey` | `#D1D1D6` | `#48484A` | `Tokens.badgeGrey` | Pair |
| `--dec` | `#5856D6` | `#7D7AFF` | `Tokens.decision` | Pair (the dark value is the increased-contrast `systemIndigo`) |
| `--dec-ink`, `--dec-soft` | `#4644B8`, `#EFEFFD` | `#BDBCFF`, `#2F2E55` | `Tokens.decisionInk`, `Tokens.decisionSoft` | Pair |
| `--act` | `#0A8270` | `#2EC5AC` | `Tokens.action` | Pair |
| `--act-ink`, `--act-soft` | `#076B5C`, `#E3F4F1` | `#7FE3D2`, `#163B35` | `Tokens.actionInk`, `Tokens.actionSoft` | Pair |
| `--done` | `#2E9E4F` | `#30D158` | `Tokens.done` | Pair (dark matches `systemGreen`) |
| `--done-ink`, `--done-soft` | `#146C2F`, `#E7F4EA` | `#84E3A0`, `#1A3A24` | `Tokens.doneInk`, `Tokens.doneSoft` | Pair |
| `--info` | `#8E8E93` | `#98989D` | `Tokens.info` | System: `NSColor.systemGray` |
| `--info-ink`, `--info-soft` | `#636366`, `#F2F2F7` | `#C7C7CC`, `#3A3A3C` | `Tokens.infoInk`, `Tokens.infoSoft` | Pair |
| `--wait` | `#D70015` | `#FF453A` | `Tokens.waiting` | Pair (also the urgent figure and the menu-bar dot) |
| `--wait-ink`, `--wait-soft` | `#A8000F`, `#FDECEE` | `#FFA59F`, `#4A1E1D` | `Tokens.waitingInk`, `Tokens.waitingSoft` | Pair |
| `--paused` | `#E8710A` | `#FF9F0A` | `Tokens.paused` | Pair (dark matches `systemOrange`) |
| `--paused-ink`, `--paused-soft` | `#A64B00`, `#FFF2E6` | `#FFC472`, `#47300F` | `Tokens.pausedInk`, `Tokens.pausedSoft` | Pair |
| `--warn-ink`, `--warn-soft` | `#8A5A00`, `#FFF4E0` | `#FFC472`, `#3B2E13` | `Tokens.warningInk`, `Tokens.warningSoft` | Pair (rules parse error) |
| `--topic-release`, `--topic-deps` | `#A2845E`, `#5E6E82` | `#C9AB80`, `#93A3B8` | `Tokens.topicColours` | Pair (sample topic icons) |
| `--menubar*` | | | none | The menu-bar icon is a template image; the system tints it |
| `--shadow-panel`, `--shadow-window` | | | none | System window and panel shadows |
| `--board`, `--desk` | `#FBFBFD`, `#D8DDE4` | `#161617`, `#1B2028` | none | Canvas only |

`--block*` and `--ask*` are older names for the waiting and paused colours. The current `Tokens.listBackground` goes away: the popover list is no longer grey.

## Boards

Each v1 board below has a dark twin, `<board>-dark`, under it on the canvas; `Main` and the mobile group are light only.

| Group | Board | Notes |
| --- | --- | --- |
| Foundations and menu bar | `Main` foundations | |
| | `mac-menubar-icons` | light and dark menu-bar strips on each board |
| Popover | `mac-popover` accordion, next up open: the blocked-on message as the full card, the rest as rows, topic and thread as a breadcrumb | |
| | `mac-popover-accordion` another row opened, next up folded back | |
| | `mac-popover-undo` actioned items held with a bar and Undo before sending | |
| | `mac-popover-steps` Do it opens the action plan | |
| | `mac-popover-item-menu` mute, move, Close leading to the open items, Dismiss and Select to dismiss | |
| | `mac-popover-dismiss` Close with open items: the thread's open items, several selected to dismiss | |
| | `mac-popover-empty` all clear, quiet hours | |
| Popups | `mac-blocking-popup` decision, waiting, urgent | |
| | `mac-blocking-stacked`, `mac-blocking-reply`, `mac-blocking-no-waiter` (Open conversation, Snooze, Clear), `mac-popup-deferrable` (action, paused, Defer), `mac-popup-rescinded` (an explicit withdrawal, then Withdrawn and Handled in the session rows) | |
| Toast and notifications | `mac-completion-toast`, `mac-native-banner` decision, agent working, `mac-native-banner-expanded`, `mac-generic-row` silent inbox rows | |
| Main window | `mac-topic` topic detail: its threads; `mac-thread` thread detail: open items and a timeline across its sessions; `mac-conversation` detail: pending action, timeline, markdown body; `mac-outcomes` answered, done, deferred, dismissed, handled in the session and withdrawn, afterwards in History and the conversation | |
| Settings | `settings-notifications` policy by type for the Mac (surface, native notification, sound, repeat sound: once, a number of times or until handled, and every N seconds), blocking-level and urgent rules, muted topics, while-you-work and Snooze length | |
| | `settings-quiet-focus` Focus status permission (`INFocusStatusCenter`); Full Disk Access only as a note for builds without the paid team | |
| | `settings-sources` per-source switch, wait-for-answers timeout | |
| | `settings-rules` rules set type, urgent and blocking, saved back to rules.toml | `settings-rules-error` |
| | `settings-phone` iPhone pane tagged Planned: "Coming with the mobile release" | |
| Later · mobile release | `ios-escalation-silent` 1: passive arrival in Notification Centre | light only; dark mode for the iPhone and Watch comes with the mobile release |
| | `ios-lock-screen` 2: the Time Sensitive ping 2 min later | `ios-lock-screen-expanded` (long press, approval: Approve, Deny, Reply), `ios-faceid` (Approve asks for Face ID), `ios-lock-screen-choices` (one button per choice, Defer, Reply, Open), `ios-lock-screen-reply` (text reply field), `ios-lock-screen-stacked` (stacked by thread), `ios-lock-screen-undo` (Undo for 3 s after a lock-screen action) |
| | `ios-escalation-handled` 3: replaced by "Approved from phone" or "Handled on Mac" | |
| | `watch` mirrored notifications with buttons; reply by dictation, Scribble or keyboard | |
| | `ios-pairing` scan the Mac's QR code; the ntfy app only as a fallback link | |
| | `ios-inbox` | `ios-inbox-undo` (actioned items held 3 s with Undo) |
| | `ios-detail` | `ios-detail-answered` |
| | `ios-history` | |
| | `mobile-settings-phone` the full iPhone settings for later: delivery (our app or the ntfy app), paired iPhones with Revoke, Apple push key (`.p8`, Key ID, Team ID, masked), ntfy server and token, Summary or Minimal, end-to-end encryption and project icon (both off), Load full messages over Tailscale (off) | `mobile-settings-pairing` |

## Undo before sending

Every action gets the same 3 s hold, in the popover and the popups on the Mac, and on the iPhone in the mobile release. Anything acted on (approve, deny, a choice, reply, done, defer, dismiss, clear) shrinks to a small box with a 3 s bar and Undo; nothing is sent while the bar runs, and Undo puts the message back where it was. On the Mac, ⌘Z undoes the last one, closing the popover does not cancel the timer, and several held actions stack, newest on top. On the lock screen, the Content Extension keeps the long-press card open with Undo as its only button; a swipe action replaces the notification with an "Approved" one carrying Undo for the same 3 s.

## Conflicts with the Phase 1 docs

None open. ADR 0007 settled Snooze (local) versus Defer (an answer), permission prompts (decision_required + blocking), the per-source switch and the "wait for answers" timeout (Settings › Sources), and rules edited in the app. Urgency is badge only, with no "URGENT:" prefix in any title (the user's choice).

Open: the undo window. `architecture.md` has the UI send answers straight to the hub actor, which closes the message on the first answer ("first answer wins"). Holding an answer for 3 s needs a decision on where the hold lives (in the app before the command is sent, or as a pending state in the hub), what the agent's own session shows meanwhile, and what happens if the answer arrives there during the window (the held answer would then get 409 and show "Already answered on <channel>"). Later, on iOS, the hold also has to fit the background time an action response gets.

Settled: descriptive titles (ADR 0010, architecture §5.5). The hub enforces them for message, conversation, topic and thread titles and summaries, so surfaces show `title` and `summary` as they arrive and never sanitise. A flagged title moves to the top of the body as an "Original title" code box and is replaced by the summary's first sentence or a fallback ("Action needed in gentbox"); the inbox why line may mention the rewrite. Claude Code permission titles follow the adapter's patterns: "Allow Bash to <description>?", "Allow editing <file name>?", "Allow fetching a page from <host>?", "Allow <Server> to <tool in words>?"; the boards use these.

## Open design questions

1. Is the agent mark (speech bubble with `>_`) right?
2. Keyboard shortcuts on the non-activating panel (⌘↩, ⌘⌫, ⌘R): make the panel key without activating the app, or a global hotkey?
3. The per-project icon override (§10) has no board yet: a Projects list in Settings, or a field in the conversation detail?
4. Finished tasks now sit in the popover list until dismissed. Should they clear on their own once read?
5. Primary buttons: white on the system accent is 4.0:1 in light and 3.6:1 in dark, under AA. The design fills them with a slightly deeper fixed blue (`--accent-fill`) that passes. Should the app do the same, or keep the system prominent button, which follows the user's accent colour?

## Tools

`tools/` rebuilds the symbol outlines, generates the dark boards, checks contrast and renders static previews.

- `match.swift` maps SF Symbol names (`names.txt`) to SF Pro private-use codepoints (`pua.txt`) by rendering both and comparing; `map.tsv` is the result, with `minus` corrected by hand to `U+10017D`.
- `extract.py` reads those glyphs from `/Library/Fonts/SF-Pro.ttf` (regular, opsz 17) into a 100-unit frame; `build-symbols.py <map.tsv> <icons.json> project/sf-symbols.js` adds the agent mark (and brand marks the boards no longer use). Needs SF Pro and the SF Symbols app installed.
- `make-dark.py` writes each v1 board's dark twin and lays out `project/canvas.json`; rerun it after editing a light board or adding one.
- `contrast.py [--markdown]` checks the light and dark pairs listed in it against WCAG AA.
- `preview.py <out-dir> [boards]` fills the icon holes and writes plain HTML for a local look (headless Chrome screenshots).
- Every symbol name was checked against `symbol_restrictions.strings`; the restricted `iphone` and `applewatch` appear only for the iPhone and the Apple Watch.
