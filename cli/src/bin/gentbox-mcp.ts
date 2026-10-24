#!/usr/bin/env node
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { readDiscoveryEnvironment } from '../discovery.ts';
import { connectToHub } from '../hub-client.ts';
import {
	connectOnFirstUse,
	createMcpServer,
	DEFAULT_ASK_TIMEOUT_SECONDS,
} from '../mcp-server.ts';
import { newConversationId, readSenderContext } from '../sender-context.ts';

// stdout carries the protocol, so every diagnostic goes to stderr.
let sender: ReturnType<typeof readSenderContext>;
try {
	sender = readSenderContext(process.env, {
		agent: 'claude-code',
		conversationId: () => newConversationId('mcp'),
	});
} catch (error) {
	process.stderr.write(
		`gentbox-mcp: ${error instanceof Error ? error.message : String(error)}\n`,
	);
	process.exit(2);
}

const server = createMcpServer({
	getClient: connectOnFirstUse(() =>
		connectToHub({
			environment: readDiscoveryEnvironment(process.env),
			identity: { agent: sender.agent, conversationId: sender.conversationId },
		}),
	),
	sender,
	defaultTimeoutSeconds: DEFAULT_ASK_TIMEOUT_SECONDS,
});

await server.connect(new StdioServerTransport());
