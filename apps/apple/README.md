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

**Checking the panel's motion.** The inbox panel is a header, the content at its natural height and a foot; its top edge is set once on opening, and only its height changes while it shows, on the same curve as the content (`Tokens.motion`, `PanelHeightMotion`). Content taller than the screen is the one exception: the panel stops at the screen and the content scrolls. In debug builds, `GENTBOX_TRACE_DIR=<folder in the app's reach>` with `-GentboxOpenInboxAtLaunch YES` makes the app drive expand, collapse, expand-another, a refresh mid-motion and a removal itself, as clicks do, then ask for an arrival, a superseding update and a waiter by writing `awaiting-<step>` (the UI test answers each). It records every display frame from the first one drawn after each change, without clicking or taking focus. `frames.csv` has the panel's left edge, top and height, the content's height and `gap`, how far the content's drawn bottom (a magenta marker) sits from the panel's (`missing` when a frame shows no marker); a PNG per frame goes beside it. `testThePanelsHeightMovesWithTheContent` and `testThePanelsTopEdgeNeverMoves` read that record and attach it to the result bundle when they fail.

