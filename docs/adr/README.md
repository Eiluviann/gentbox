# Architecture decision records

One file per decision: context, decision, consequences, alternatives. A superseded record stays and points to its successor.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-hub-placement.md) | Rust core in-process in the Mac menu-bar app; the same crates as the headless `gentboxd` | Accepted |
| [0002](0002-ingest-transport.md) | Loopback HTTP + Unix socket, bearer token, long-poll for answers | Accepted |
| [0003](0003-message-schema.md) | Message schema v1: camelCase JSON, sender-declared class, correlation keys | Superseded in part by 0007 |
| [0004](0004-policy-engine.md) | Pure, table-tested policy function; rules in TOML; no model | Superseded in part by 0007 |
| [0005](0005-ios-transport.md) | Phone push through ntfy now (ntfy.sh default, answers via an answer topic), APNs adapter later | Superseded by 0006 |
| [0006](0006-phone-apns.md) | Own iOS app over APNs from the Mac with the user's key; ntfy as answer channel and fallback | Accepted |
| [0007](0007-message-type-urgency-blocking.md) | `type` + `urgent` + `blocking` replace the class; Defer and rescind | Accepted |
