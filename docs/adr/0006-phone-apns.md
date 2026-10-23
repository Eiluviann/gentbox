# 0006 Phone transport: own iOS app over APNs, ntfy for answers and fallback

Status: Accepted, 2026-10-23. Supersedes [0005](0005-ios-transport.md). Amended by [0007](0007-message-type-urgency-blocking.md): urgency and blocking replace the class, so Time Sensitive follows `urgent`, categories gain Done and Defer, and rescinds remove delivered notifications.

## Context

The user wants notifications on the iPhone and Watch with our own UI and icon, a title, a description and real action buttons. ADR 0005 chose ntfy because the app was signed by a free Personal Team, which cannot push to its own app. Only the paid Apple Developer Program gives all of that, and the user has now chosen it ("Guess I will bite the bullet and buy the apple dev sub"). The project principle still holds: no shared infrastructure that other users depend on, every user brings their own keys; running something for yourself is allowed but never required (architecture, top).

Enrolment can take a day or two, and other people may build the app without a membership, so the free path must keep working.

## Decision

### Delivery: APNs straight from the Mac

- The `apns` adapter in `notifier-relay` sends from the Mac to Apple's push service, `api.push.apple.com` (production) or `api.sandbox.push.apple.com` (development builds), over HTTP/2 on port 443 (2197 as a firewall fallback) [APNs-send].
- **Auth: token-based.** An ES256 JWT signed with the user's own APNs key (`.p8`), header `kid` = key id, claims `iss` = team id and `iat`. The `.p8`, key id and team id are entered once in Settings and kept in the Keychain. The hub refreshes the JWT no more often than every 20 min and no less often than every 60 min, since APNs rejects a token older than an hour (403 `ExpiredProviderToken`) and refreshing too often (429 `TooManyProviderTokenUpdates`) [APNs-token, APNs-responses].
- **Our own iOS app** (bundle `com.eiluviann.agentnotifier`, signed by the user's paid team) receives the pushes. No relay, no server: this works only for people who build and sign the app with their own team, because an APNs key only reaches apps of its own team.
- **Device tokens** reach the hub through pairing (below). Each token is stored with its environment (`development` for Xcode builds, `production` for TestFlight). The hub drops a token on 410 `Unregistered` or `ExpiredToken`, and on 400 `BadDeviceToken` or `DeviceTokenNotForTopic` after one retry against the other environment. It retries 429 and 5xx with backoff [APNs-responses].

### What a push carries (4 KB cap)

APNs accepts at most 4,096 bytes of JSON per notification [APNs-payload]. The hub builds the payload and trims the body excerpt last until the whole payload fits (UTF-8 safe):

| Key | Content | Budget |
| --- | --- | --- |
| `aps.alert.title` | The title phrase by type, "URGENT: " prefixed when urgent ("Your input is needed", "Task complete", …) | ~30 B |
| `aps.alert.subtitle` | Project · conversation title | ≤ 120 B |
| `aps.alert.body` | The message summary | ≤ 400 chars |
| `aps.category` | `approval`, `question`, `choice`, `action`, `completion`, `info`, each with a `-defer` variant for `deferrable` messages | small |
| `aps.thread-id` | Conversation id, so the phone groups by session | ≤ 64 B |
| `aps.interruption-level` | `passive`, `active` or `time-sensitive` (below) | small |
| `aps.sound`, `aps.relevance-score`, `aps.mutable-content: 1` | | small |
| `m` | Hub message id | 26 B |
| `a` | Up to 3 answer actions: id, label, kind, single-use action token | ~3 × 180 B |
| `i` | Icon reference: content hash and, for a project icon, a short-lived fetch URL (below) | ~150 B |
| `x` | Body excerpt, trimmed to fit | the rest, typically 2–3 KB |

`apns-collapse-id` (≤ 64 bytes) is the hub message id of the slot, so a superseding message replaces the notification in place [APNs-send].

### Actionable notifications

- The app registers fixed categories at launch:
  - `approval` (`decision_required` with approve/deny): Approve, Deny, Reply.
  - `question` (`decision_required` without them): Reply, Open.
  - `choice`: Reply, Open, plus the message's choices.
  - `action` (`action_required`): Done, Open.
  - `completion`: Open.
  - `info`: no actions.

  Each answerable category has a `-defer` variant that adds Defer for `deferrable` messages. Snooze is not a push button: it stays notifier-local (ADR 0007).
- **Approve requires an unlocked device** (`.authenticationRequired`): the system asks for Face ID before the app sees the tap, so a locked phone cannot approve anything [APNs-auth-required]. Deny and snooze do not.
- **Reply** is a `UNTextInputNotificationAction`: type, dictate or Scribble a reply from the lock screen or the Watch.
- **Choices** are per message, but categories are static. The optional Notification Content Extension sets the expanded notification's buttons at display time with `notificationActions` (iOS 12+) [Content-ext]. Without the extension a `choice` message offers Reply and Open, and the reply text is matched against the choice labels.
- **Handling a tap.** Actions run in the background (no `.foreground` option): the app posts the action token (plus the reply text) to the user's ntfy answer topic and finishes. The hub redeems it exactly as in ADR 0005. No inbound connection to the Mac, no VPN.

### Interruption levels, Focus and silent-then-ping

- **Urgent items are Time Sensitive.** They use `interruption-level: time-sensitive`, which the paid team's Time Sensitive Notifications capability allows; it breaks through Focus unless the user turns that off for the app. `critical` needs a separate Apple approval and is not used [APNs-payload].
- **Silent-then-ping** (architecture §6.2) maps to:
  - the silent push: `interruption-level: passive`, no sound.
  - the escalation: the same `apns-collapse-id` at the audible level (`time-sensitive` when urgent, `active` otherwise) with a sound.

  Spike S2 verifies that a collapsed replacement alerts again. If it does not, the escalation uses a new collapse id and a background push removes the silent one.
- **Answered elsewhere or rescinded.** A background push (`content-available`, priority 5) asks the app to remove the delivered notification (`removeDeliveredNotifications`); the app confirms through the answer topic. Without confirmation after 60 s (iOS throttles background pushes), the hub replaces it through its collapse id with a passive "Handled on Mac" or "Withdrawn" version (ADR 0007).

### Icons: Notification Service Extension

- Every push sets `mutable-content: 1`. The Notification Service Extension (about 30 s per notification [NSE]) attaches the icon as the thumbnail.
- **Agent icons** are bundled in the app.
- **Project icons** come from a cache in the app's App Group container, keyed by content hash. On a cache miss the extension downloads the icon once from a short-lived attachment the hub uploaded to the user's ntfy server. That is the user's own service under the bring-your-own-keys principle; no project-hosted asset is involved.
- If the extension runs out of time the notification shows without the thumbnail. Title, body and buttons are in the payload itself.
- **Option, off by default: project icon as the big icon.** With the Communication Notifications capability (paid) the extension can donate an `INSendMessageIntent` with the project icon as the sender avatar, so the project icon replaces the app icon, which keeps a small badge. Apple designed this for person-to-person messages, so it is a user setting.
- **Option, off by default: end-to-end encrypted content.** The extension decrypts an AES-GCM-sealed summary, excerpt and action list with a key exchanged at pairing. Apple then sees only the title phrase. The cost is about a third of the payload budget for base64.

### Notification Content Extension (optional)

The extension provides the expanded long-press view: the body excerpt rendered as markdown, the project and conversation, and dynamic choice buttons. It ships with the iOS app, but nothing depends on it.

### Watch

- **Mirroring.** With no watchOS app, the Watch mirrors the iPhone's notifications when the phone is locked and the Watch is worn. Category actions appear as buttons, and Reply offers dictation, Scribble and the keyboard.
- **Not on the Watch.** The Content Extension does not run there. Whether the Service Extension's image attachment appears on the Watch is unverified (Spike S2).
- **Watch app.** A watchOS app with custom notification scenes is Planned, not part of Phase 3.

### Pairing (no network path needed)

1. Settings › Phone shows a QR code. It holds the ntfy server, the answer topic, a pairing secret and, when E2E is on, the content key.
2. The iOS app scans it.
3. The app posts a registration (device token, environment, app version, HMAC over them with the pairing secret) to the answer topic.
4. The hub verifies and stores it, then confirms with a push.
5. The app re-registers whenever iOS issues a new device token.
6. Revoking a device in Settings deletes its token and rotates the pairing secret.

### Tailscale: optional, for full bodies only

Everything above works without a network path to the Mac. With Tailscale on both devices, the iOS app can also load full bodies beyond the 4 KB excerpt, and the inbox history, from the hub's remote listener (architecture §10).

### ntfy: answer channel always, delivery as fallback

- The ntfy answer topic carries answers and registrations in every setup.
- **ntfy delivery** (ADR 0005's design, unchanged) stays as a fallback: until the paid membership is active, and for anyone who builds the app without one. Settings › Phone picks "Our iOS app (APNs)" or "ntfy app" as the delivery channel. When an APNs push fails for good (no valid device token), the hub falls back to ntfy delivery if it is configured, and shows the failure in the menu bar.

## Consequences

- **The full feature set on the phone and Watch:** our icon and UI, title, subtitle, summary, approve/deny/choices/reply, Time Sensitive through Focus. Still no server required.
- **The Mac gains too:**
  - `INFocusStatusCenter` replaces the Full Disk Access workaround, since the Communication Notifications capability is now available.
  - Time Sensitive works for the Mac's own blocking notifications.
- **Signing moves to the paid team.** Development profiles last a year instead of 7 days, and Apple Development certificates last a year. The Personal Team recipe stays documented for the ntfy fallback.
- **New moving parts:**
  - JWT handling and the HTTP/2 client in Rust.
  - Two app extensions (service, content).
  - Pairing, with device-token lifecycle and sandbox vs production environments.
- **Apple's push service carries the notification content in transit**, unless the E2E option is on.
- **Notifications reach only people who build and sign the app themselves.** A store build for others would need a central push relay, which the principle rules out.

## Alternatives

- **Keep ntfy as the only delivery (ADR 0005):** free, but the ntfy app's icon and UI, no text reply, and an app whose development is paused.
- **Pushover:** a polished app with end-to-end encryption and an emergency priority, but no answer buttons (ADR 0005 › Alternatives).
- **A push relay server so store builds work for everyone:** would be shared infrastructure that other users depend on, which the principle rules out. A relay for your own devices only is allowed but gains nothing here, since the Mac already pushes directly.
- **Certificate-based APNs auth (`.p12`):** expires yearly and is per app; token auth with one `.p8` covers all the team's apps and environments [APNs-token].

## Sources

- [APNs-send] Sending notification requests to APNs: https://developer.apple.com/documentation/usernotifications/sending-notification-requests-to-apns
- [APNs-token] Establishing a token-based connection to APNs: https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns
- [APNs-responses] Handling notification responses from APNs: https://developer.apple.com/documentation/usernotifications/handling-notification-responses-from-apns
- [APNs-payload] Generating a remote notification: https://developer.apple.com/documentation/usernotifications/generating-a-remote-notification
- [APNs-auth-required] UNNotificationActionOptions.authenticationRequired: https://developer.apple.com/documentation/usernotifications/unnotificationactionoptions/authenticationrequired
- [NSE] Modifying content in newly delivered notifications: https://developer.apple.com/documentation/usernotifications/modifying-content-in-newly-delivered-notifications
- [Content-ext] Customizing the appearance of notifications: https://developer.apple.com/documentation/usernotificationsui/customizing-the-appearance-of-notifications; NSExtensionContext.notificationActions: https://developer.apple.com/documentation/foundation/nsextensioncontext/notificationactions
