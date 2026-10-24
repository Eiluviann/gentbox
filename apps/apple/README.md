# Apple apps

The macOS menu-bar app (v1). It is the hub (architecture §3): it runs the Rust core in-process through `gentbox-ffi` and serves the local API the agents post to. The iOS app follows in the mobile release (M2).

| Path | What |
| --- | --- |
| `project.yml` | XcodeGen spec; `xcodegen generate` writes `Gentbox.xcodeproj` (gitignored) |
| `Signing.xcconfig`, `Signing.personal-team.xcconfig`, `Signing/*.entitlements` | signing for the paid team, a Personal Team and ad hoc; see [`docs/signing.md`](../../docs/signing.md) |
| `scripts/build-xcframework.sh` | builds `GentboxCoreFFI.xcframework` and the Swift bindings from `core/crates/gentbox-ffi` |
| `scripts/test.sh` | the Apple gate: unit tests, the XCUITest smoke and the package tests, signed ad hoc |
| `scripts/render-app-icon.swift` | renders the app icon from the Gentbox mark |
| `GentboxCore/` | Swift package: the generated bindings, `InboxModel`, `PresenceMonitor`, `StateFolder` |
| `Gentbox/` | the app: status item and inbox panel (`MenuBar/`, `MenuBarInbox/`, following the `mac-popover` boards), `LoginItem` |
| `GentboxTests/`, `GentboxUITests/` | entitlement and menu-bar mark tests; the UI smoke |

```bash
apps/apple/scripts/build-xcframework.sh
cd apps/apple && xcodegen generate && xcodebuild -project Gentbox.xcodeproj -scheme Gentbox -configuration Debug -derivedDataPath build build
```

**Threading.** The hub calls its delegate on the hub thread. `MainActorDelegate` hops every event to the main actor and returns at once; nothing there calls the hub synchronously, which would deadlock it. Every hub call from Swift is `async` and runs on the FFI crate's own Tokio runtime.

**Agent marks.** `Gentbox/Assets.xcassets/AgentClaude` and `AgentCodex` are the Claude and Codex marks, as Zed's agent registry ships them (`~/Library/Application Support/Zed/external_agents/registry/icons`), recoloured to template images. They are their owners' trademarks and only name the agent on a row; any other agent gets a terminal symbol.

**The inbox is a panel, not a window.** `InboxPanel` is borderless and non-activating, at the pop-up menu level, on every Space and outside the window cycle. It reports `AXSystemDialog`, never `AXStandardWindow`, so tiling window managers such as AeroSpace leave it alone (`InboxPanelTests`; checked against AeroSpace 0.20.2-Beta, which lists no Gentbox window). Popups and toasts (PR 4) follow the same rule.

**The panel holds still; the card moves.** The inbox panel's window is set once on opening and never resizes: a transparent column from just above the card down to the bottom of the screen. A transparent window still takes its own clicks, so the panel takes the mouse only while the pointer is over the card (mouse-moved monitors while it shows); below the card, clicks and scrolls reach the windows under it. The card in it is a header, the content at its natural height and a foot. The surface is card-sized with its own rounded corners; a hairline ring and a nine-slice shadow image, drawn once, follow it from layer-hosting views. Only its height changes while it shows, in the same Core Animation transaction as the content's own motion. Content taller than the screen is the one exception: the card stops at the screen and the content scrolls. The card spawns with a fade and a scale from 0.96 about its top centre (180 ms, ease-out) and despawns to 0.98 (120 ms, ease-in); a re-open mid-despawn turns back where it is, and Reduce Motion fades only.

**The content is AppKit and Core Animation** (ADR [0012](../../docs/adr/0012-menu-bar-inbox-appkit.md)). `InboxContentView` reads the model into a snapshot on each change, updates only the rows and sections whose inputs changed, lays them out by hand and animates the change from the presented values (`LayerMotion`). Native controls cost a SwiftUI graph each on macOS 26, so a card's buttons and reply field are built ahead, one per quiet run-loop turn (`IdleWork`), and kept drawn at zero opacity until the card shows them. Every button is a native push button (`InboxButton`: the accent bezel for the primary action, `hasDestructiveAction` for Deny, borderless for text actions). The expanded card keeps the collapsed row's text sizes. The reply box spans the card's width at one line, takes a line more as the text is about to wrap, stops at three and scrolls inside after that (`ReplyField`); its growth moves with the card's height. SwiftUI is left only for the `App` entry point.

**Measuring it.** `Interactions` marks signposted spans (subsystem `com.eiluviann.gentbox`, category `Interaction`: Open, Expand, Select, Action) from a click to the next display frame, for Instruments and `XCTOSSignpostMetric`. Debug builds, and builds made with `SWIFT_ACTIVE_COMPILATION_CONDITIONS=GENTBOX_HOOKS` for measuring Release, carry two hooks; Release and the Personal Team recipe carry neither.

- `GENTBOX_TRACE_DIR=<…/Gentbox/UITraces/<name>>` with `-GentboxOpenInboxAtLaunch YES`: the app drives expand, collapse, expand-another, a refresh mid-motion and a removal itself, as clicks do, asks for an arrival, a superseding update and a waiter by writing `awaiting-<step>` (the UI test answers each), then despawns, spawns and re-opens mid-despawn. It records every display frame from the first one drawn after each change, without clicking or taking focus. `frames.csv` has the card's left edge, top and height, the content's height, `gap` (how far the content's drawn bottom, a magenta marker, sits from the card's), and the presented opacity and scale; `steps.csv` has each step's height before and the delay to its first sample; a PNG per frame goes beside them. The trace tests read that record and attach it when they fail. The marker shows only while tracing.
- `GENTBOX_DEMO_DIR=<folder>`: runs one command per `*.cmd` file dropped there (open, close, expand, collapse, reply, dismiss, select, a click-through backdrop, frame pacing with a 120 Hz display link), for screen recordings and pacing measurements that need no clicks. Pacing counts a frame as missed when it lands later than the previous frame's target time, so ProMotion dropping to 48 Hz while nothing moves is not counted. `GENTBOX_REDUCE_MOTION=0|1` overrides the system setting for both.

Measuring rules, because Instruments fills the disk: record with `--time-limit` and the smallest template that answers the question; check `df -h /` first and skip a run under 15 GB free; after each run export the tables you need, delete the `.trace` bundle and the `instruments*.ktrace` scratch it leaves in the user temp folder (`getconf DARWIN_USER_TEMP_DIR`, 4.5–13.7 GB a run); keep only the exported CSV or XML.
