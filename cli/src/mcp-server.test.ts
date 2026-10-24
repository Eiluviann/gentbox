import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js';
import { describe, expect, it, vi } from 'vitest';
import type { HubClient } from './hub-client.ts';
import { connectOnFirstUse, createMcpServer } from './mcp-server.ts';
import { fail, succeed } from './result.ts';
import { TEST_SENDER } from './test-support/command-fixture.ts';
import { actionResponse, ingestResponse } from './test-support/hub-fixtures.ts';
import { stubHubClient } from './test-support/stub-hub-client.ts';

async function connectClient(hubClient: HubClient): Promise<Client> {
	const server = createMcpServer({
		getClient: async () => succeed(hubClient),
		sender: TEST_SENDER,
		defaultTimeoutSeconds: 300,
	});
	const [clientSide, serverSide] = InMemoryTransport.createLinkedPair();
	await server.connect(serverSide);
	const client = new Client({ name: 'test-client', version: '0.0.0' });
	await client.connect(clientSide);
	return client;
}

describe('the MCP server over a real protocol session', () => {
	it('answers tools/list with the eight tools and object schemas', async () => {
		const client = await connectClient(stubHubClient());
		const { tools } = await client.listTools();
		expect(tools.map((tool) => tool.name)).toEqual([
			'notify',
			'ask',
			'wait',
			'rescind',
			'create_topic',
			'close_topic',
			'create_thread',
			'close_thread',
		]);
		for (const tool of tools) {
			expect(tool.inputSchema.type).toBe('object');
			expect(tool.description?.length).toBeGreaterThan(40);
		}
		const notify = tools.find((tool) => tool.name === 'notify');
		expect(JSON.stringify(notify?.inputSchema)).toContain(
			'"required":["steps"]',
		);
		expect(client.getInstructions()).toContain('plain-words summary');
	});

	it('round-trips an ask through tools/call', async () => {
		const postMessage = vi.fn<HubClient['postMessage']>(async () =>
			succeed({ wasCreated: true, record: ingestResponse() }),
		);
		const getResponse = async () =>
			succeed(
				actionResponse('answered', {
					actionId: 'approve',
					actionKind: 'approve',
					answeredVia: 'mac',
				}),
			);
		const client = await connectClient(
			stubHubClient({ postMessage, getResponse }),
		);
		const result = await client.callTool({
			name: 'ask',
			arguments: { title: 'Allow Bash to run the ingest tests?' },
		});
		const first = Array.isArray(result.content) ? result.content[0] : undefined;
		expect(
			first?.type === 'text' ? JSON.parse(first.text) : undefined,
		).toMatchObject({ status: 'answered', actionKind: 'approve' });
		expect(postMessage.mock.calls[0]?.[0]).toMatchObject({
			awaitsResponse: true,
		});
	});

	it('returns invalid arguments as an error result the model can read', async () => {
		const client = await connectClient(stubHubClient());
		const result = await client.callTool({
			name: 'notify',
			arguments: { type: 'action_required', title: 'Sign in to npm' },
		});
		expect(result.isError).toBe(true);
	});

	it('cancels the wait when the client cancels the request', async () => {
		const rescindMessage = vi.fn<HubClient['rescindMessage']>(async () =>
			succeed(actionResponse('rescinded')),
		);
		let didStartPolling: () => void = () => undefined;
		const polling = new Promise<void>((resolve) => {
			didStartPolling = resolve;
		});
		const getResponse: HubClient['getResponse'] = (_id, _seconds, signal) =>
			new Promise((resolve) => {
				didStartPolling();
				signal?.addEventListener('abort', () =>
					resolve(succeed(actionResponse('pending'))),
				);
			});
		const hubClient = stubHubClient({
			postMessage: async () =>
				succeed({ wasCreated: true, record: ingestResponse() }),
			getResponse,
			rescindMessage,
		});
		const client = await connectClient(hubClient);
		const controller = new AbortController();
		const call = client.callTool(
			{ name: 'ask', arguments: { title: 'Allow Bash to run the tests?' } },
			undefined,
			{ signal: controller.signal },
		);
		await polling;
		controller.abort();
		await expect(call).rejects.toThrow();
		await vi.waitFor(() => expect(rescindMessage).toHaveBeenCalledOnce());
	});
});

describe('connectOnFirstUse', () => {
	it('connects once and keeps the client', async () => {
		const connect = vi.fn(async () => succeed(stubHubClient()));
		const getClient = connectOnFirstUse(connect);
		await getClient();
		await getClient();
		expect(connect).toHaveBeenCalledOnce();
	});

	it('tries again after a failed attempt, so starting Gentbox later works', async () => {
		const connect =
			vi.fn<
				() => Promise<
					| ReturnType<typeof succeed<HubClient>>
					| ReturnType<typeof fail<string>>
				>
			>();
		connect
			.mockResolvedValueOnce(fail('not running'))
			.mockResolvedValueOnce(succeed(stubHubClient()));
		const getClient = connectOnFirstUse(connect);
		expect((await getClient()).ok).toBe(false);
		expect((await getClient()).ok).toBe(true);
		expect(connect).toHaveBeenCalledTimes(2);
	});
});
