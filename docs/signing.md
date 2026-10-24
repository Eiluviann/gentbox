# Signing the Mac app

Gentbox's Mac app is sandboxed and runs the hub in-process: it keeps `hub.db`, `hub.lock`, `rules.toml` and the `API/` discovery folder in a state folder that depends on how the build is signed (architecture §3). Three signings work:

| Signing | Status | Identifiers | Gives up |
| --- | --- | --- | --- |
| **Paid team** (the shipped app) | Pending: the Apple Developer membership is still in identity verification | `com.eiluviann.gentbox`, App Group `group.com.eiluviann.gentbox` | nothing |
| **Personal Team** (a free Apple ID) | **The working default until the membership is active**; verified end to end on 2026-10-24 | `com.eiluviann.gentbox.personal`, App Group `<TEAM>.com.eiluviann.gentbox` | Time Sensitive and Communication Notifications, so no Focus status from `INFocusStatusCenter` |
| **Ad hoc** ("Sign to Run Locally", no Apple ID) | The default with no setup; what the tests and CI use | `com.eiluviann.gentbox`, no App Group | the above, the App Group, and the Unix socket (TCP only) |

The team id never goes into a committed file: the repository is public. It lives in `apps/apple/Signing.local.xcconfig`, which is gitignored, and every recipe below writes only that file.

## Recipe: Personal Team

This follows Vaultday's recipe (`docs/signing.md` there), which has the troubleshooting for certificates.

1. Xcode › Settings › Accounts › **+** › Apple Account. Sign in with your Apple ID; Xcode creates "<Your Name> (Personal Team)".
2. Select the team, **Manage Certificates… › + › Apple Development**, unless Xcode already made one.
3. Check that it is a valid signing identity; if `-v` shows none, the Worldwide Developer Relations G3 intermediate is missing (Vaultday's recipe says how to add it):

   ```bash
   security find-identity -v -p codesigning
   ```

4. Find the team id: the Organizational Unit of the certificate, not the 10 characters in its name.

   ```bash
   security find-certificate -c "Apple Development" -p | openssl x509 -noout -subject
   ```

5. Write `apps/apple/Signing.local.xcconfig` (gitignored):

   ```
   DEVELOPMENT_TEAM = ABCDE12345
   #include "Signing.personal-team.xcconfig"
   ```

6. Build:

   ```bash
   apps/apple/scripts/build-xcframework.sh && cd apps/apple && xcodegen generate && xcodebuild -project Gentbox.xcodeproj -scheme Gentbox -configuration Debug -derivedDataPath build build
   ```

7. Run `apps/apple/build/Build/Products/Debug/Gentbox.app`. The mark appears in the menu bar; on the first launch the app registers itself as a login item and macOS says "“Gentbox” will open automatically when you log in".

**Why identifiers of its own.** Bundle ids and `group.` App Groups are unique across all teams, and Xcode registers what it signs. A Personal Team signing with the shipped names would hold them, and the paid team could not register them later. `Signing.personal-team.xcconfig` therefore uses `com.eiluviann.gentbox.personal` and a macOS-style App Group prefixed by the team id, which is never registered (`REGISTER_APP_GROUPS = NO`) and opens without a prompt.

**No weekly rebuild.** `Signing/PersonalTeam.entitlements` claims nothing restricted (the sandbox, the local server and the team-prefixed App Group), so the build carries no provisioning profile at all and does not expire after 7 days; only the Apple Development certificate does, after a year. Vaultday needs the weekly rebuild because it claims `keychain-access-groups`; Gentbox v1 keeps no Keychain items.

**What it gives up.** Free accounts cannot claim Time Sensitive Notifications or Communication Notifications (architecture §11), and claiming a restricted entitlement without a profile gets the app killed at launch, so the Personal Team set leaves both out:

- Blocking native notifications (PR 4) arrive as ordinary ones, which a Focus can silence; the persistent popup still shows.
- Focus status comes from a Shortcuts automation running `gentbox focus on|off` (PR 4 documents it), not from `INFocusStatusCenter`.

## Recipe: paid team (once the membership is active)

1. Sign in with the account that holds the membership (Xcode › Settings › Accounts) and note its team id as above.
2. Write `apps/apple/Signing.local.xcconfig`:

   ```
   DEVELOPMENT_TEAM = <paid team id>
   CODE_SIGN_IDENTITY = Apple Development
   GENTBOX_ENTITLEMENTS = Signing/Full.entitlements
   ```

3. Build with `-allowProvisioningUpdates`, so `xcodebuild` registers `com.eiluviann.gentbox` and `group.com.eiluviann.gentbox` and makes the development profile (it lasts a year):

   ```bash
   cd apps/apple && xcodegen generate && xcodebuild -project Gentbox.xcodeproj -scheme Gentbox -configuration Debug -derivedDataPath build -allowProvisioningUpdates build
   ```

4. Check that the embedded profile grants both notification capabilities (the PR 3 acceptance item still open):

   ```bash
   security cms -D -i apps/apple/build/Build/Products/Debug/Gentbox.app/Contents/embedded.provisionprofile | plutil -extract Entitlements xml1 -o - -
   ```

   It must list `com.apple.developer.usernotifications.time-sensitive` and `com.apple.developer.usernotifications.communication`.

The paid build's state starts fresh in its own App Group: nothing moves over from a Personal Team build (a different app to macOS).

## Recipe: ad hoc

Nothing to configure: without `Signing.local.xcconfig`, `Signing.xcconfig` signs with `CODE_SIGN_IDENTITY = -` and `Signing/AdHoc.entitlements`. The build claims no App Group, so the state goes to `~/Library/Application Support/Gentbox`, which the entitlements' home-relative exception opens. The sandbox refuses a Unix socket in that folder, so the hub serves the local API on loopback TCP only; clients fall back to TCP after the `hello` proof (architecture §8), as they would for an over-long socket path.

An unsigned build (`CODE_SIGNING_ALLOWED=NO`) runs unsandboxed with the same folder, and with the socket.

## Which state folder a build uses

The app reads the App Groups its running signature claims (`SecTask`), not only what Info.plist names, so a project set up for a team but built unsigned never tries to open a group it cannot (`StateFolder` in `GentboxCore`).

| Build | State folder | Local API |
| --- | --- | --- |
| Paid team | `~/Library/Group Containers/group.com.eiluviann.gentbox` | `API/hub.sock` and loopback |
| Personal Team | `~/Library/Group Containers/<TEAM>.com.eiluviann.gentbox` | `API/hub.sock` and loopback |
| Ad hoc | `~/Library/Application Support/Gentbox` | loopback only |
| UI tests (`GENTBOX_STATE_DIR`) | `~/Library/Application Support/Gentbox/UITests`, emptied at each launch | loopback only |

Every launch logs the choice:

```bash
/usr/bin/log show --last 5m --info --style compact --predicate 'subsystem == "com.eiluviann.gentbox"'
```

(`log` alone is a zsh builtin.) What a build claims:

```bash
codesign -d --entitlements - --xml apps/apple/build/Build/Products/Debug/Gentbox.app | plutil -p -
```

**One hub per user.** Two builds on the same state folder cannot run at once: the second one says "another hub is running". Builds with different folders (a Personal Team build and an ad hoc build) can, and each serves its own `API/` folder; the CLI uses the first it finds (architecture §8).

## Tests and CI

`apps/apple/scripts/test.sh` is the Apple gate. It always signs ad hoc with no team, whatever `Signing.local.xcconfig` says, so it behaves the same on a developer Mac and on CI:

- The UI smoke needs a signature to launch the app and its runner, so CI does not use `CODE_SIGNING_ALLOWED=NO`; ad hoc needs no certificate.
- Xcode sandboxes the UI test runner with read-only access to the disk. The app keeps its state in `UITests/` and empties it at launch; the test only reads the discovery files and posts with `curl`.
- The test targets build without the hardened runtime: an ad hoc runner has no team, and library validation would refuse to load the test bundle.
- With "Automatically hide and show the menu bar" on, the status item sits above the screen and XCUITest's synthetic pointer never reveals it. The smoke then launches the app with `-GentboxOpenInboxAtLaunch YES`, which opens the inbox panel at launch without taking the keyboard, and still checks the status item; with the menu bar visible (CI) it clicks the item.

## Login item

The app registers itself with `SMAppService.mainApp` on its first launch (it is the hub, so it should run whenever you are logged in), and the gear menu's **Open at Login** changes it afterwards. A development build registers the path it was launched from, so a build in a worktree's `build/` folder stops opening at login once that folder is gone; turn it off first in the gear menu or in System Settings › General › Login Items. UI tests pass `-GentboxSkipLoginItem YES` and never register.
