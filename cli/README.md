# @gentbox/cli

`gentbox` (CLI) and `gentbox-mcp` (stdio MCP server): how an agent sends messages to Gentbox and waits for answers. Both find the running hub through its `API/` folder (`GENTBOX_API_DIR`, else the App Group container, `~/Library/Application Support/Gentbox/API`, `$XDG_STATE_HOME/gentbox/API`), prefer the Unix socket, and use TCP only after the hub's `hello` proof checks out.

```bash
pnpm --filter @gentbox/cli build            # dist/bin/gentbox.js, dist/bin/gentbox-mcp.js
gentbox send --type task_completed --title "Ingest auth accepts rotated tokens; 12 tests pass"
gentbox ask --title "Allow Bash to run the ingest tests?" --timeout 120   # exit 0 answered, 3 deferred, 4 no answer
claude mcp add gentbox -- node /path/to/cli/dist/bin/gentbox-mcp.js
```

Identity is self-asserted: `GENTBOX_AGENT` (default `cli`; `claude-code` for the MCP server), `GENTBOX_CONVERSATION` (default one per folder; the MCP server makes one per process). The sender token cannot answer or list messages, so there is no `answer` or `list` command.

The integration test (`src/integration.test.ts`) runs against a real `gentboxd` when `core/target/debug/gentboxd` (or `GENTBOXD_BIN`) exists: `cd core && cargo build -p gentboxd`.
