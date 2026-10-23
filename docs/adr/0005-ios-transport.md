# 0005 Phone transport: ntfy now, APNs later

Status: Superseded by [0006](0006-phone-apns.md), 2026-10-23. Its ntfy answer topic and ntfy delivery live on in 0006 as the answer channel and the fallback. (Earlier the same day: revised with the user's answers, ntfy.sh by default, answers through ntfy instead of Tailscale.)

## Context

Blocking items should reach the iPhone and Watch, and be answerable there. The app is signed by a free Personal Team: no APNs, no CloudKit, no Time Sensitive entitlement, iOS profiles expiring every 7 days. Answers from the phone must reach the Mac hub, which is not on the public internet. The user prefers a hosted service for ease and does not use Tailscale. Project principle: no hosted infrastructure; every user brings their own keys (architecture, top).

## Decision

- **Push through ntfy**, ntfy.sh by default, self-hosted supported, via the `Relay` trait in `notifier-relay`. The ntfy iOS app (App Store, with its own push entitlements) shows the notification, mirrored to the Watch.
- **Content**: a fixed title phrase per class ("URGENT: Action required", "Your input is needed", "Task complete", "Goal complete"), the message summary, and the project or agent icon as an image attachment. `minimal` mode (phrase only) and the `sensitive` flag are available; `body` and `cwd` never leave the Mac.
- **Silent-then-ping**: while the Mac is in use the phone gets the notification silently at once and pings only if it is still unaddressed after 2 min; when the Mac is away it pings at once.
- **Answers back through ntfy**: `http` buttons POST an HMAC-signed, single-use, 30-minute action token to a second, secret **answer topic**; the hub holds an outbound subscription to it and redeems the token. No inbound port, no VPN.
- **Own iOS app** (optional, PR 6) as an inbox and detail viewer over Tailscale, for full content and text replies.
- **APNs adapter later** behind the same trait, once the paid membership exists: the Mac sends directly to Apple's push service with the user's own APNs key, to the user's own build of the iOS app. No relay; therefore only for people who sign the app themselves.

## Consequences

- Works today with no paid account, no server and no VPN.
- ntfy.sh sees the title phrase, the summary, the icon and the action tokens. Tokens are single-use and bound to one message and action, so the exposure is limited to what the summary says. Self-hosting removes the third party.
- The ntfy iOS app is the weakest link: the ntfy FAQ calls it bare-bones and somewhat buggy with development paused, and known issues include notifications silently stopping until the subscription is re-added [ntfy-faq, ntfy-known-issues]. `http` buttons give no visible result, ignore `clear`, and open the app on tap. Spike S2 (PR 5) tests it on the user's phone and Watch before the adapter is built on.
- Up to 3 buttons on the phone and no text input there.

## Alternatives

- **Pushover** ([pushover.net/api](https://pushover.net/api), [pricing](https://pushover.net/pricing)): $4.99 one-time per platform after a 30-day trial; 10,000 messages a month free; maintained apps; end-to-end encryption with a key kept on the device; an emergency priority that repeats every ≥ 30 s (up to 3 h, 50 retries) until acknowledged, with a receipt the hub can poll outbound and a cancel call. But no answer buttons: one supplementary link per message, and "acknowledge" only stops the repeats; it does not tell the agent anything. Approvals would need that link to lead to the own iOS app (Tailscale); a hosted page is ruled out by the no-infrastructure principle. Bring-your-own-keys fits: each user creates their own Pushover application token. The best fallback if Spike S2 shows the ntfy iOS app is unreliable, and a good fit for "URGENT" escalation; it fits the same `Relay` trait.
- **Answers over Tailscale** into a remote listener on the Mac (the first version of this ADR): direct and private, but needs a VPN app on the phone and the Mac; kept only for the optional iOS app.
- **Own iOS app with background polling**: iOS gives no reliable background execution without push; pings would arrive late or never.
- **Bark** (open-source iOS push app over its own APNs relay): Time Sensitive and critical support but no action buttons (unverified).
- **iMessage or e-mail**: no structured actions, and sending iMessages programmatically needs Apple Events into Messages.
- **Pay for the Developer Program now**: the clean path (APNs, Time Sensitive, Focus status) but a cost the user chose to defer.
