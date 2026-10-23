# 0005 Phone transport: ntfy now, APNs later

Status: Accepted, 2026-10-23

## Context

Blocking items should reach the iPhone and Watch when you are away. The app is signed by a free Personal Team: no APNs, no CloudKit, no Time Sensitive entitlement, iOS profiles expiring every 7 days. Answers from the phone must reach the Mac hub, which is not on the public internet.

## Decision

- **Push through ntfy** (ntfy.sh by default, self-hosted supported) via the `Relay` trait in `notifier-relay`. The ntfy iOS app (App Store, with its own push entitlements) shows the notification, mirrored to the Watch.
- **Minimal payload by default**: agent name and a class phrase, no message title or content. `title` and `full` modes are opt-in; `full` is recommended only with self-hosted ntfy, where only a poll id transits ntfy.sh.
- **Answers over Tailscale**: ntfy `http` action buttons POST to a remote action endpoint bound to the Mac's Tailscale address, off by default, accepting only HMAC-signed, single-use, short-lived action tokens carried in the request body.
- **Own iOS app** as an inbox and detail viewer over the same Tailscale listener (HTTPS with the MagicDNS certificate), for text replies and full content.
- **APNs adapter later** behind the same trait, once the paid membership exists; it sends directly to the own iOS app with native action categories.

## Consequences

- Works today with no paid account and no server to run; phone actions need Tailscale on the phone.
- The ntfy iOS app is the weakest link: bare-bones, development paused, known issues with notifications silently stopping; `http` actions give no visible result, ignore `clear`, and open the app on tap. The hub republishes the notification with the outcome (Spike S2 checks whether iOS honours updates).
- Up to 3 buttons on the phone and no text input there; replies go through the own iOS app.
- ntfy.sh limits (250 messages a day anonymously) are comfortable with digests; a self-hosted server removes them.

## Alternatives

- **Own iOS app with background polling**: iOS gives no reliable background execution without push; pings would arrive late or never.
- **Pushover** (one-time purchase per platform): reliable iOS and Watch delivery and an emergency priority that re-alerts until acknowledged, with an acknowledgement callback, but no custom action buttons (unverified beyond its public API docs; not researched in depth). The strongest contingency if the ntfy iOS app proves unreliable in Spike S2; it fits the same `Relay` trait.
- **Bark** (open-source iOS push app over its own APNs relay): Time Sensitive and critical support but no action buttons (unverified).
- **iMessage or e-mail**: no structured actions, and sending iMessages programmatically needs Apple Events into Messages.
- **Pay for the Developer Program now**: the clean path (APNs, Time Sensitive, Focus status) but a cost the user chose to defer.
