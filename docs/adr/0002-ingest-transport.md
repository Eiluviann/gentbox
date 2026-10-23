# 0002 Ingest transport

Status: Accepted, 2026-10-23

## Context

Senders are hook scripts (short-lived processes, TypeScript or shell), an MCP server, a CLI and arbitrary agents. A blocking sender must be able to wait for your answer and fall back to its own prompt on timeout. Vaultday already exposes a loopback API with a token file that its CLI discovers.

## Decision

- **HTTP/1.1 JSON on `127.0.0.1:<random port>`** and the same router on a **Unix socket** (`<state>/API/hub.sock`, 0600). `axum` on `tokio`.
- **Bearer token** (256-bit) in `<state>/API/token`, 0600 in a 0700 folder, discovered like Vaultday's (`GET /v1/hello` proves the pairing; `AGENT_NOTIFIER_API_DIR` overrides).
- **Answers by long-poll**: `GET /v1/messages/{id}/response?wait=≤55`. The sender loops until its own deadline. The hub replies `pending` at the end of each wait, so no intermediary or client timeout ever sees a hung request.
- `Host` header check, JSON-only content type, no CORS: a browser cannot use the port even if it guesses it.
- A **separate remote listener** for the phone, bound only to the Tailscale address and off by default, serving only `/r/v1/…` routes (ADR 0005).

## Consequences

- `curl` is a complete client, which keeps the hook and any third-party agent trivial and makes integration tests plain shell.
- Long-poll costs one idle connection per waiting sender; a handful at most.
- The hook pays a TCP connect per call (sub-millisecond on loopback); Unix socket available when it matters.

## Alternatives

- **WebSocket or SSE** for answers: push instead of poll, but a hook script would need a WS client and reconnection logic for no gain at this scale.
- **Unix socket only**: not every agent runtime can speak HTTP over a Unix socket (Windows is Planned), while every one can reach loopback TCP.
- **gRPC**: schema-first and streaming, but heavy for hook scripts and adds codegen to every client.
- **Files in a watched folder**: no answer path and no validation feedback.
