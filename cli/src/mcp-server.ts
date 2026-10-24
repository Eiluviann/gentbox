import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import {
	CallToolRequestSchema,
	ListToolsRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';
import type { HubClient } from './hub-client.ts';
import { runTool, type ToolDeps } from './mcp-tool-handlers.ts';
import { TOOL_SPECS } from './mcp-tool-schemas.ts';
import type { Result } from './result.ts';

export const MCP_SERVER_VERSION = '0.1.0';
export const DEFAULT_ASK_TIMEOUT_SECONDS = 300;

const INSTRUCTIONS =
	"Gentbox is the user's inbox for AI agents. Use `ask` when you need the user's decision and `notify` for what they should know; do not use them for chatter. " +
	'Write every `title` as a plain-words summary, never a command, path or URL. ' +
	'An action_required message needs `steps`. Heed the warnings in tool results.';

/**
 * The stdio MCP server: `notify`, `ask`, `rescind`, `create_topic`,
 * `close_topic`, `create_thread` and `close_thread`. Tool listing and calls
 * live here; what each tool does is in mcp-tool-handlers.ts.
 */
export function createMcpServer(deps: ToolDeps): Server {
	const server = new Server(
		{ name: 'gentbox', version: MCP_SERVER_VERSION },
		{ capabilities: { tools: {} }, instructions: INSTRUCTIONS },
	);
	server.setRequestHandler(ListToolsRequestSchema, async () => ({
		tools: TOOL_SPECS.map(({ name, description, inputSchema }) => ({
			name,
			description,
			inputSchema,
		})),
	}));
	server.setRequestHandler(CallToolRequestSchema, async (request, extra) =>
		runTool(request.params.name, request.params.arguments, deps, extra.signal),
	);
	return server;
}

/**
 * Connects on the first tool call and keeps the client. A failed attempt is
 * tried again on the next call, and so is a client that reports a lost
 * connection (the hub quit, or restarted on a new port): the endpoint is
 * discovered and, over TCP, proven again.
 */
export function connectOnFirstUse(
	connect: (onConnectionLost: () => void) => Promise<Result<HubClient, string>>,
): () => Promise<Result<HubClient, string>> {
	let pending: Promise<Result<HubClient, string>> | undefined;
	return () => {
		if (pending !== undefined) return pending;
		const attempt: Promise<Result<HubClient, string>> = connect(() => {
			if (pending === attempt) pending = undefined;
		}).then((connected) => {
			if (!connected.ok && pending === attempt) pending = undefined;
			return connected;
		});
		pending = attempt;
		return attempt;
	};
}
