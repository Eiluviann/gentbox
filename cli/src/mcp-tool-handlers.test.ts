import type { CallToolResult } from '@modelcontextprotocol/sdk/types.js';
import { describe, expect, it, vi } from 'vitest';
import type { HubClient } from './hub-client.ts';
import { formatWarnings, runTool, type ToolDeps } from './mcp-tool-handlers.ts';
import { fail, succeed } from './result.ts';
import { TEST_SENDER } from './test-support/command-fixture.ts';
import {
	actionResponse,
	ingestResponse,
	MESSAGE_ID,
	THREAD_ID,
	TOPIC_ID,
} from './test-support/hub-fixtures.ts';
import { stubHubClient } from './test-support/stub-hub-client.ts';

function depsFor(
	client: HubClient,
	overrides: Partial<ToolDeps> = {},
): ToolDeps {
	return {
		getClient: async () => succeed(client),
		sender: TEST_SENDER,
		defaultTimeoutSeconds: 300,
		...overrides,
	};
}

function textOf(result: CallToolResult): string {
	return result.content
		.map((block) => (block.type === 'text' ? block.text : ''))
		.join('\n');
}

function jsonOf(result: CallToolResult): Record<string, unknown> {
	const first = result.content[0];
	if (first?.type !== 'text') throw new Error('no text block');
	return JSON.parse(first.text);
}

function fakeClock() {
	let milliseconds = 0;
	return {
		now: () => milliseconds,
		advance: (seconds: number) => {
			milliseconds += seconds * 1000;
		},
		sleep: (delay: number) => {
			milliseconds += delay;
			return Promise.resolve();
		},
	};
}

const MUTED_WARNING = {
	code: 'muted',
	message:
		"Delivered to the inbox without a ping: topic 'Release 1.4' is muted",
} as const;

const WARNING = {
	code: 'title_looks_like_command',
	field: 'title',
	message:
		'The title looked like a command and was moved into the body; it now reads "Allow Bash to run the tests?". Write titles as plain-word summaries.',
} as const;

describe('notify', () => {
	it('sends the declared type, urgency and blocking and returns the ids', async () => {
		const postMessage = vi.fn<HubClient['postMessage']>(async () =>
			succeed({
				wasCreated: true,
				record: ingestResponse({
					type: 'task_completed',
					blocking: 'non_blocking',
					shouldWait: false,
				}),
			}),
		);
		const result = await runTool(
			'notify',
			{
				type: 'task_completed',
				title: 'Ingest auth accepts rotated tokens; 12 tests pass',
				scope: 'goal',
				topic: 'repo:gentbox',
				topicTitle: 'gentbox',
			},
			depsFor(stubHubClient({ postMessage })),
			undefined,
		);
		expect(result.isError).toBeUndefined();
		expect(postMessage.mock.calls[0]?.[0]).toMatchObject({
			type: 'task_completed',
			scope: 'goal',
			event: 'mcp.notify',
			topic: { key: 'repo:gentbox', title: 'gentbox' },
		});
		expect(postMessage.mock.calls[0]?.[0].awaitsResponse).toBeUndefined();
		expect(jsonOf(result)).toMatchObject({
			messageId: MESSAGE_ID,
			type: 'task_completed',
		});
	});

	it('passes the hub warnings back verbatim', async () => {
		const postMessage = async () =>
			succeed({
				wasCreated: true,
				record: ingestResponse({ warnings: [WARNING] }),
			});
		const result = await runTool(
			'notify',
			{ type: 'generic', title: 'cargo test -p ingest' },
			depsFor(stubHubClient({ postMessage })),
			undefined,
		);
		expect(textOf(result)).toContain(WARNING.message);
		expect(textOf(result)).toContain('[title_looks_like_command] title:');
	});

	it('tells the model its notification was delivered but muted', async () => {
		const postMessage = async () =>
			succeed({
				wasCreated: true,
				record: ingestResponse({
					shouldWait: false,
					warnings: [MUTED_WARNING],
				}),
			});
		const result = await runTool(
			'notify',
			{ type: 'task_completed', title: 'Release notes are drafted' },
			depsFor(stubHubClient({ postMessage })),
			undefined,
		);
		expect(jsonOf(result)).toMatchObject({
			messageId: MESSAGE_ID,
			state: 'open',
			muted: true,
		});
		expect(textOf(result)).toContain(MUTED_WARNING.message);
		expect(textOf(result)).toContain('[muted]');
	});

	it('refuses action_required without steps and never contacts the hub', async () => {
		const getClient = vi.fn(async () => succeed(stubHubClient()));
		const result = await runTool(
			'notify',
			{ type: 'action_required', title: 'Sign in to npm' },
			depsFor(stubHubClient(), { getClient }),
			undefined,
		);
		expect(result.isError).toBe(true);
		expect(textOf(result)).toContain(
			'`steps` is required when `type` is `action_required`',
		);
		expect(getClient).not.toHaveBeenCalled();
	});

	it('sends steps with an action_required message', async () => {
		const postMessage = vi.fn<HubClient['postMessage']>(async () =>
			succeed({
				wasCreated: true,
				record: ingestResponse({ type: 'action_required' }),
			}),
		);
		const result = await runTool(
			'notify',
			{
				type: 'action_required',
				urgent: true,
				blocking: 'deferrable',
				title: 'Sign in to npm so I can publish',
				steps: [{ text: 'Sign in to npm', detail: '```bash\nnpm login\n```' }],
			},
			depsFor(stubHubClient({ postMessage })),
			undefined,
		);
		expect(result.isError).toBeUndefined();
		expect(postMessage.mock.calls[0]?.[0]).toMatchObject({
			type: 'action_required',
			urgent: true,
			blocking: 'deferrable',
			steps: [{ text: 'Sign in to npm' }],
		});
	});

	it('reports a hub refusal with its code and pointer', async () => {
		const postMessage = async () =>
			fail({
				kind: 'http',
				status: 422,
				code: 'schema_invalid',
				message: 'bad',
				pointer: '/title',
			} as const);
		const result = await runTool(
			'notify',
			{ type: 'generic', title: 'x' },
			depsFor(stubHubClient({ postMessage })),
			undefined,
		);
		expect(result.isError).toBe(true);
		expect(textOf(result)).toBe('schema_invalid at /title: bad');
	});

	it('says how to start Gentbox when the hub cannot be reached', async () => {
		const result = await runTool(
			'notify',
			{ type: 'generic', title: 'x' },
			depsFor(stubHubClient(), {
				getClient: async () =>
					fail('Gentbox is not running or cannot be reached.'),
			}),
			undefined,
		);
		expect(result).toMatchObject({ isError: true });
		expect(textOf(result)).toContain('Gentbox is not running');
	});
});

describe('ask', () => {
	function askingClient(
		getResponse: HubClient['getResponse'],
		ingest = ingestResponse(),
	) {
		const postMessage = vi.fn<HubClient['postMessage']>(async () =>
			succeed({ wasCreated: true, record: ingest }),
		);
		const rescindMessage = vi.fn<HubClient['rescindMessage']>(async () =>
			succeed(actionResponse('rescinded')),
		);
		return {
			client: stubHubClient({ postMessage, rescindMessage, getResponse }),
			postMessage,
			rescindMessage,
		};
	}

	it('returns the chosen answer', async () => {
		const { client, postMessage } = askingClient(async () =>
			succeed(
				actionResponse('answered', {
					actionId: 'choice-1',
					actionKind: 'choice',
					value: 'Use SQLite',
					answeredVia: 'mac',
				}),
			),
		);
		const result = await runTool(
			'ask',
			{
				title: 'Which store should the hub use?',
				choices: ['Use SQLite', 'Use Postgres'],
				allowReply: true,
			},
			depsFor(client),
			undefined,
		);
		expect(jsonOf(result)).toMatchObject({
			status: 'answered',
			actionKind: 'choice',
			value: 'Use SQLite',
		});
		const sent = postMessage.mock.calls[0]?.[0];
		expect(sent).toMatchObject({
			type: 'decision_required',
			blocking: 'blocking',
			awaitsResponse: true,
			event: 'mcp.ask',
		});
		expect(sent?.actions?.map((action) => action.kind)).toEqual([
			'choice',
			'choice',
			'reply',
			'open',
		]);
	});

	it('defaults to Approve and Deny and lets the agent pick deferrable', async () => {
		const { client, postMessage } = askingClient(async () =>
			succeed(actionResponse('answered', { actionKind: 'approve' })),
		);
		await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?', blocking: 'deferrable' },
			depsFor(client),
			undefined,
		);
		expect(
			postMessage.mock.calls[0]?.[0].actions?.map((action) => action.kind),
		).toEqual(['approve', 'deny', 'open']);
		expect(postMessage.mock.calls[0]?.[0].blocking).toBe('deferrable');
	});

	it('returns deferred without withdrawing the message', async () => {
		const { client, rescindMessage } = askingClient(async () =>
			succeed(actionResponse('deferred')),
		);
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?', blocking: 'deferrable' },
			depsFor(client),
			undefined,
		);
		expect(jsonOf(result)).toMatchObject({ status: 'deferred' });
		expect(textOf(result)).toContain(
			'carry on with the work that needs no input',
		);
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('returns "no answer yet" and leaves the message open when the timeout passes', async () => {
		const clock = fakeClock();
		const { client, rescindMessage } = askingClient(async (_id, seconds) => {
			clock.advance(seconds);
			return succeed(actionResponse('pending'));
		});
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?', timeoutSeconds: 60 },
			depsFor(client, { wait: clock }),
			undefined,
		);
		expect(result.isError).toBeUndefined();
		expect(jsonOf(result)).toMatchObject({
			status: 'no_answer',
			reason: 'timed_out',
		});
		expect(jsonOf(result)).not.toHaveProperty('messageWithdrawn');
		expect(textOf(result)).toContain('the message stays open');
		expect(textOf(result)).toContain('`wait`');
		expect(textOf(result)).toContain('`rescind`');
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('uses the default timeout when the agent gives none', async () => {
		const getResponse = vi.fn<HubClient['getResponse']>(async () =>
			succeed(actionResponse('answered', { actionKind: 'approve' })),
		);
		const { client } = askingClient(getResponse);
		await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?' },
			depsFor(client, { defaultTimeoutSeconds: 20 }),
			undefined,
		);
		expect(getResponse.mock.calls[0]?.[1]).toBe(20);
	});

	it('leaves the message open when the call is cancelled', async () => {
		const controller = new AbortController();
		const { client, rescindMessage } = askingClient(async () => {
			controller.abort();
			return succeed(actionResponse('pending'));
		});
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?' },
			depsFor(client),
			controller.signal,
		);
		expect(jsonOf(result)).toMatchObject({
			status: 'no_answer',
			reason: 'cancelled',
		});
		expect(jsonOf(result)).not.toHaveProperty('messageWithdrawn');
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('leaves the message open when the wait fails', async () => {
		const { client, rescindMessage } = askingClient(async () =>
			fail({
				kind: 'http',
				status: 500,
				code: 'internal',
				message: 'boom',
				pointer: undefined,
			}),
		);
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?' },
			depsFor(client),
			undefined,
		);
		expect(result.isError).toBe(true);
		expect(textOf(result)).toContain('still open');
		expect(textOf(result)).not.toContain('withdrawn');
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('names a rule as the source of a mute, not a topic or thread', async () => {
		const ruleWarning = {
			code: 'muted',
			message:
				"Delivered to the inbox without a ping: rule 'Mute prompts' mutes it",
		} as const;
		const { client } = askingClient(
			vi.fn<HubClient['getResponse']>(),
			ingestResponse({ shouldWait: false, warnings: [ruleWarning] }),
		);
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?' },
			depsFor(client),
			undefined,
		);
		const instruction = String(jsonOf(result).instruction);
		expect(instruction).toContain("rule 'Mute prompts' mutes it");
		expect(instruction).not.toContain('topic or thread');
	});

	it('tells the model its ask was merged into another open message, and not to rescind that one', async () => {
		const postMessage = async () =>
			succeed({
				wasCreated: false,
				record: ingestResponse({ shouldWait: false }),
			});
		const rescindMessage = vi.fn<HubClient['rescindMessage']>();
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?' },
			depsFor(stubHubClient({ postMessage, rescindMessage })),
			undefined,
		);
		expect(jsonOf(result)).toMatchObject({
			status: 'no_answer',
			reason: 'merged',
			messageId: MESSAGE_ID,
		});
		const instruction = String(jsonOf(result).instruction);
		expect(instruction).toContain('merged into');
		expect(instruction).toContain('do not rescind');
		expect(instruction).not.toContain('`wait`');
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('returns muted at once: delivered without a ping, still open, collect it with wait', async () => {
		const getResponse = vi.fn<HubClient['getResponse']>();
		const { client, rescindMessage } = askingClient(
			getResponse,
			ingestResponse({ shouldWait: false, warnings: [MUTED_WARNING] }),
		);
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?' },
			depsFor(client),
			undefined,
		);
		expect(result.isError).toBeUndefined();
		expect(jsonOf(result)).toMatchObject({
			status: 'no_answer',
			reason: 'muted',
			messageId: MESSAGE_ID,
		});
		expect(jsonOf(result)).not.toHaveProperty('messageWithdrawn');
		const text = textOf(result);
		expect(text).toContain('delivered to the inbox');
		expect(text).toContain('muted');
		expect(text).toContain("wasn't pinged");
		expect(text).toContain('`wait`');
		expect(text).toContain('`rescind`');
		expect(text).toContain('resolved_elsewhere');
		expect(text).toContain(MUTED_WARNING.message);
		expect(getResponse).not.toHaveBeenCalled();
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('does not wait when nobody can answer, and leaves the message open', async () => {
		const getResponse = vi.fn<HubClient['getResponse']>();
		const { client, rescindMessage } = askingClient(
			getResponse,
			ingestResponse({ shouldWait: false }),
		);
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?' },
			depsFor(client),
			undefined,
		);
		expect(jsonOf(result)).toMatchObject({
			status: 'no_answer',
			reason: 'nobody_can_answer_now',
		});
		expect(jsonOf(result)).not.toHaveProperty('messageWithdrawn');
		expect(getResponse).not.toHaveBeenCalled();
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('explains a withdrawn or dismissed question', async () => {
		const { client } = askingClient(async () =>
			succeed(actionResponse('rescinded', { rescindReason: 'superseded' })),
		);
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?' },
			depsFor(client),
			undefined,
		);
		expect(jsonOf(result)).toMatchObject({
			status: 'rescinded',
			rescindReason: 'superseded',
		});
	});

	it('says an item the hub cleared was handled in the session, not withdrawn', async () => {
		const { client } = askingClient(async () =>
			succeed(
				actionResponse('rescinded', { rescindReason: 'handled_in_session' }),
			),
		);
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?' },
			depsFor(client),
			undefined,
		);
		expect(jsonOf(result)).toMatchObject({
			status: 'rescinded',
			rescindReason: 'handled_in_session',
		});
		const text = textOf(result);
		expect(text).toContain('handled in the session');
		expect(text).toContain('not withdrawn');
	});

	it('says a withdrawal is a withdrawal', async () => {
		const { client } = askingClient(async () =>
			succeed(actionResponse('rescinded', { rescindReason: 'superseded' })),
		);
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?' },
			depsFor(client),
			undefined,
		);
		expect(textOf(result)).not.toContain('handled in the session');
	});

	it('passes the hub warnings back verbatim, also on an answer', async () => {
		const { client } = askingClient(
			async () =>
				succeed(actionResponse('answered', { actionKind: 'approve' })),
			ingestResponse({ warnings: [WARNING] }),
		);
		const result = await runTool(
			'ask',
			{ title: 'cargo test -p ingest' },
			depsFor(client),
			undefined,
		);
		expect(textOf(result)).toContain(WARNING.message);
	});

	it('requires steps for an action_required ask and offers no Approve or Deny', async () => {
		const { client, postMessage } = askingClient(async () =>
			succeed(actionResponse('answered', { actionKind: 'done' })),
		);
		const missing = await runTool(
			'ask',
			{ type: 'action_required', title: 'Sign in to npm so I can publish' },
			depsFor(client),
			undefined,
		);
		expect(missing.isError).toBe(true);
		const sent = await runTool(
			'ask',
			{
				type: 'action_required',
				title: 'Sign in to npm so I can publish',
				steps: [{ text: 'Sign in to npm' }],
			},
			depsFor(client),
			undefined,
		);
		expect(jsonOf(sent)).toMatchObject({
			status: 'answered',
			actionKind: 'done',
		});
		expect(
			postMessage.mock.calls[0]?.[0].actions?.map((action) => action.kind),
		).toEqual(['open']);
	});

	it('reports a lost connection as an error and says the answer is unknown', async () => {
		const clock = fakeClock();
		const { client } = askingClient(async () =>
			fail({ kind: 'transport', message: 'ECONNRESET' }),
		);
		const result = await runTool(
			'ask',
			{ title: 'Allow Bash to run the tests?' },
			depsFor(client, { wait: clock }),
			undefined,
		);
		expect(result.isError).toBe(true);
		expect(textOf(result)).toContain('answer is unknown');
	});
});

describe('wait', () => {
	it('collects the answer to a deferred question', async () => {
		const getResponse = vi.fn<HubClient['getResponse']>(async () =>
			succeed(actionResponse('answered', { actionKind: 'approve' })),
		);
		const result = await runTool(
			'wait',
			{ messageId: MESSAGE_ID, timeoutSeconds: 30 },
			depsFor(stubHubClient({ getResponse })),
			undefined,
		);
		expect(jsonOf(result)).toMatchObject({
			status: 'answered',
			messageId: MESSAGE_ID,
			actionKind: 'approve',
		});
		expect(getResponse.mock.calls[0]?.[1]).toBe(30);
	});

	it('says a message the hub cleared was handled in the session, not withdrawn', async () => {
		const getResponse = async () =>
			succeed(
				actionResponse('rescinded', { rescindReason: 'handled_in_session' }),
			);
		const result = await runTool(
			'wait',
			{ messageId: MESSAGE_ID },
			depsFor(stubHubClient({ getResponse })),
			undefined,
		);
		expect(jsonOf(result)).toMatchObject({
			status: 'rescinded',
			rescindReason: 'handled_in_session',
		});
		expect(textOf(result)).toContain('handled in the session');
		expect(textOf(result)).toContain('not withdrawn');
	});

	it('leaves the message open when nothing arrives in time', async () => {
		const clock = fakeClock();
		const rescindMessage = vi.fn<HubClient['rescindMessage']>();
		const getResponse: HubClient['getResponse'] = async (_id, seconds) => {
			clock.advance(seconds);
			return succeed(actionResponse('pending'));
		};
		const result = await runTool(
			'wait',
			{ messageId: MESSAGE_ID, timeoutSeconds: 60 },
			depsFor(stubHubClient({ getResponse, rescindMessage }), { wait: clock }),
			undefined,
		);
		expect(jsonOf(result)).toMatchObject({
			status: 'no_answer',
			reason: 'timed_out',
		});
		expect(jsonOf(result)).not.toHaveProperty('messageWithdrawn');
		expect(textOf(result)).toContain('stays open');
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('rejects an id that is not a message id', async () => {
		const result = await runTool(
			'wait',
			{ messageId: '../topics' },
			depsFor(stubHubClient()),
			undefined,
		);
		expect(result.isError).toBe(true);
	});
});

describe('rescind', () => {
	it('withdraws by message id, defaulting to resolved_elsewhere', async () => {
		const rescindMessage = vi.fn<HubClient['rescindMessage']>(async () =>
			succeed(
				actionResponse('rescinded', { rescindReason: 'resolved_elsewhere' }),
			),
		);
		const result = await runTool(
			'rescind',
			{ messageId: MESSAGE_ID },
			depsFor(stubHubClient({ rescindMessage })),
			undefined,
		);
		expect(rescindMessage).toHaveBeenCalledWith(
			MESSAGE_ID,
			'resolved_elsewhere',
		);
		expect(jsonOf(result)).toMatchObject({ status: 'rescinded' });
	});

	it('withdraws by correlation keys, defaulting to superseded', async () => {
		const rescindByCorrelationKeys = vi.fn<
			HubClient['rescindByCorrelationKeys']
		>(async () => succeed([MESSAGE_ID]));
		const result = await runTool(
			'rescind',
			{ correlationKeys: ['plan'], reason: 'obsolete' },
			depsFor(stubHubClient({ rescindByCorrelationKeys })),
			undefined,
		);
		expect(rescindByCorrelationKeys).toHaveBeenCalledWith({
			reason: 'obsolete',
			correlationKeys: ['plan'],
		});
		expect(jsonOf(result)).toEqual({ rescinded: [MESSAGE_ID] });
	});

	it('names the hub error when the message is already closed', async () => {
		const rescindMessage = async () =>
			fail({
				kind: 'http' as const,
				status: 409,
				code: 'already_closed',
				message: 'the message is already answered',
				pointer: undefined,
			} as const);
		const result = await runTool(
			'rescind',
			{ messageId: MESSAGE_ID },
			depsFor(stubHubClient({ rescindMessage })),
			undefined,
		);
		expect(result).toMatchObject({ isError: true });
		expect(textOf(result)).toContain('already_closed');
	});
});

describe('topic and thread tools', () => {
	const topic = {
		id: TOPIC_ID,
		key: 'release-1.4',
		title: 'Release 1.4',
		implicit: false,
		status: 'open',
		createdBy: { agent: 'claude-code' },
		createdAt: '2026-10-23T18:30:00Z',
		updatedAt: '2026-10-23T18:30:00Z',
	} as const;
	const thread = {
		id: THREAD_ID,
		topicId: TOPIC_ID,
		key: 'phase-3',
		title: 'Phase 3',
		implicit: false,
		status: 'open',
		createdBy: { agent: 'claude-code' },
		createdAt: '2026-10-23T18:30:00Z',
		updatedAt: '2026-10-23T18:30:00Z',
	} as const;

	it('create_topic records the creating conversation', async () => {
		const createTopic = vi.fn<HubClient['createTopic']>(async () =>
			succeed({ wasCreated: true, record: topic }),
		);
		const result = await runTool(
			'create_topic',
			{ key: 'release-1.4', title: 'Release 1.4', style: { color: 'orange' } },
			depsFor(stubHubClient({ createTopic })),
			undefined,
		);
		expect(createTopic).toHaveBeenCalledWith({
			key: 'release-1.4',
			title: 'Release 1.4',
			style: { color: 'orange' },
			conversation: { id: 'session-1' },
		});
		expect(jsonOf(result)).toMatchObject({
			wasCreated: true,
			topic: { id: TOPIC_ID },
		});
	});

	it("create_topic hands the hub's title-guard refusal to the model verbatim", async () => {
		const message =
			'the topic title looks like a command; write a plain-words name';
		const createTopic = async () =>
			fail({
				kind: 'http',
				status: 422,
				code: 'title_looks_like_command',
				message,
				pointer: undefined,
			} as const);
		const result = await runTool(
			'create_topic',
			{ key: 'k', title: 'cargo test' },
			depsFor(stubHubClient({ createTopic })),
			undefined,
		);
		expect(result.isError).toBe(true);
		expect(textOf(result)).toContain(message);
	});

	it('close_topic resolves a key and can withdraw open messages', async () => {
		const listTopics = async () => succeed([topic]);
		const closeTopic = vi.fn<HubClient['closeTopic']>(async () =>
			succeed({ ...topic, status: 'closed' }),
		);
		await runTool(
			'close_topic',
			{ topic: 'release-1.4', rescindOpenMessages: true, reason: 'obsolete' },
			depsFor(stubHubClient({ listTopics, closeTopic })),
			undefined,
		);
		expect(closeTopic).toHaveBeenCalledWith(TOPIC_ID, {
			rescindOpenMessages: true,
			reason: 'obsolete',
		});
	});

	it.each([
		['close_topic', { topic: 'release-1.4' }, 'closeTopic', 'topic'],
		['close_thread', { thread: THREAD_ID }, 'closeThread', 'thread'],
	] as const)(
		'%s turns a refused close into a clear message with the count',
		async (name, input, method, entity) => {
			const listTopics = async () => succeed([topic]);
			const refused = vi.fn(async () =>
				fail({
					kind: 'http' as const,
					status: 409,
					code: 'has_open_messages',
					message: 'still open',
					pointer: undefined,
					count: 4,
				}),
			);
			const result = await runTool(
				name,
				input,
				depsFor(stubHubClient({ listTopics, [method]: refused })),
				undefined,
			);
			expect(result.isError).toBe(true);
			const text = textOf(result);
			expect(text).toContain(`${entity} was not closed`);
			expect(text).toContain('4 open messages');
			expect(text).toContain('only your own');
		},
	);

	it('close_topic says the General topic cannot be closed', async () => {
		const listTopics = async () => succeed([topic]);
		const closeTopic = async () =>
			fail({
				kind: 'http' as const,
				status: 409,
				code: 'general_topic_cannot_close',
				message: 'no',
				pointer: undefined,
			});
		const result = await runTool(
			'close_topic',
			{ topic: 'release-1.4' },
			depsFor(stubHubClient({ listTopics, closeTopic })),
			undefined,
		);
		expect(result.isError).toBe(true);
		expect(textOf(result)).toContain("General topic can't be closed");
	});

	it('create_thread can stand for the current conversation', async () => {
		const createThread = vi.fn<HubClient['createThread']>(async () =>
			succeed({ wasCreated: true, record: thread }),
		);
		await runTool(
			'create_thread',
			{
				key: 'session:web',
				title: 'Release notes page',
				topic: 'release-1.4',
				topicTitle: 'Release 1.4',
				standsForCurrentConversation: true,
			},
			depsFor(stubHubClient({ createThread })),
			undefined,
		);
		expect(createThread).toHaveBeenCalledWith({
			topic: { key: 'release-1.4', title: 'Release 1.4' },
			key: 'session:web',
			title: 'Release notes page',
			summary: undefined,
			kind: 'conversation',
			conversation: { id: 'session-1' },
		});
	});

	it('close_thread takes an id, or a key with its topic', async () => {
		const closeThread = vi.fn<HubClient['closeThread']>(async () =>
			succeed({ ...thread, status: 'closed' }),
		);
		const client = stubHubClient({
			closeThread,
			listTopics: async () => succeed([topic]),
			listThreads: async () => succeed([thread]),
		});
		await runTool(
			'close_thread',
			{ thread: THREAD_ID },
			depsFor(client),
			undefined,
		);
		await runTool(
			'close_thread',
			{ thread: 'phase-3', topic: 'release-1.4' },
			depsFor(client),
			undefined,
		);
		expect(closeThread.mock.calls.map((call) => call[0])).toEqual([
			THREAD_ID,
			THREAD_ID,
		]);
		const needsTopic = await runTool(
			'close_thread',
			{ thread: 'phase-3' },
			depsFor(client),
			undefined,
		);
		expect(needsTopic.isError).toBe(true);
		expect(textOf(needsTopic)).toContain('name its topic');
	});
});

describe('formatWarnings', () => {
	it('is absent without warnings', () => {
		expect(formatWarnings(undefined)).toBeUndefined();
		expect(formatWarnings([])).toBeUndefined();
	});
});

describe('runTool', () => {
	it('rejects an unknown tool', async () => {
		expect(
			await runTool('answer', {}, depsFor(stubHubClient()), undefined),
		).toMatchObject({ isError: true });
	});

	it('turns an unexpected exception into an error result instead of throwing', async () => {
		const postMessage = async () => {
			throw new Error('boom');
		};
		const result = await runTool(
			'notify',
			{ type: 'generic', title: 'x' },
			depsFor(stubHubClient({ postMessage })),
			undefined,
		);
		expect(result).toMatchObject({ isError: true });
		expect(textOf(result)).toContain('boom');
	});
});
