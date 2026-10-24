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
| `Gentbox/` | the app: status item and popover (`MenuBar/`, `MenuBarInbox/`), `LoginItem` |
| `GentboxTests/`, `GentboxUITests/` | entitlement and menu-bar mark tests; the UI smoke |

```bash
apps/apple/scripts/build-xcframework.sh
cd apps/apple && xcodegen generate && xcodebuild -project Gentbox.xcodeproj -scheme Gentbox -configuration Debug -derivedDataPath build build
```

**Threading.** The hub calls its delegate on the hub thread. `MainActorDelegate` hops every event to the main actor and returns at once; nothing there calls the hub synchronously, which would deadlock it. Every hub call from Swift is `async` and runs on the FFI crate's own Tokio runtime.
