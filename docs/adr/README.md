# Architecture decision records

One file per decision: context, decision, consequences, alternatives. A superseded record stays and points to its successor.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-hub-placement.md) | Rust core in-process in the Mac menu-bar app; the same crates as the headless `agent-notifierd` | Accepted |
| [0002](0002-ingest-transport.md) | Loopback HTTP + Unix socket, bearer token, long-poll for answers | Accepted |
| [0003](0003-message-schema.md) | Message schema v1: camelCase JSON, sender-declared class, correlation keys | Accepted |
| [0004](0004-policy-engine.md) | Pure, table-tested policy function; rules in TOML; no model | Accepted |
| [0005](0005-ios-transport.md) | Phone push through ntfy now, APNs adapter later | Accepted |
