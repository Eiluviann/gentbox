import { describe, expect, it, vi } from 'vitest';
import type { HubClient } from '../hub-client.ts';
import type { ThreadRecord, TopicRecord } from '../message-types.ts';
import { fail, succeed } from '../result.ts';
import { createCommandFixture } from '../test-support/command-fixture.ts';
import { THREAD_ID, TOPIC_ID } from '../test-support/hub-fixtures.ts';
import { stubHubClient } from '../test-support/stub-hub-client.ts';
import { EXIT } from './command-context.ts';
import { runThread } from './thread.ts';
import { runTopic } from './topic.ts';

const TOPIC: TopicRecord = {
	id: TOPIC_ID,
	key: 'release-1.4',
	title: 'Release 1.4',
	implicit: false,
	status: 'open',
	createdBy: { agent: 'claude-code' },
	createdAt: '2026-10-23T18:30:00Z',
	updatedAt: '2026-10-23T18:30:00Z',
};

const THREAD: ThreadRecord = {
	id: THREAD_ID,
	topicId: TOPIC_ID,
	key: 'phase-3',
	title: 'Phase 3',
	implicit: false,
	status: 'open',
	createdBy: { agent: 'claude-code' },
	createdAt: '2026-10-23T18:30:00Z',
	updatedAt: '2026-10-23T18:30:00Z',
};

describe('gentbox topic', () => {
	it('creates a topic with style hints and records the creating conversation', async () => {
		const createTopic = vi.fn<HubClient['createTopic']>(async () =>
			succeed({ wasCreated: true, record: TOPIC }),
		);
		const fixture = createCommandFixture(stubHubClient({ createTopic }));
		const code = await runTopic(
			[
				'create',
				'release-1.4',
				'--title',
				'Release 1.4',
				'--summary',
				'Ships Friday',
				'--symbol',
				'shippingbox',
				'--color',
				'orange',
			],
			fixture.context,
		);
		expect(code).toBe(EXIT.ok);
		expect(createTopic).toHaveBeenCalledWith({
			key: 'release-1.4',
			title: 'Release 1.4',
			summary: 'Ships Friday',
			kind: undefined,
			style: { symbol: 'shippingbox', color: 'orange' },
			conversation: { id: 'session-1' },
		});
		expect(fixture.stdoutJson()).toMatchObject({ id: TOPIC_ID });
	});

	it('lists topics by status', async () => {
		const listTopics = vi.fn<HubClient['listTopics']>(async () =>
			succeed([TOPIC]),
		);
		const fixture = createCommandFixture(stubHubClient({ listTopics }));
		expect(await runTopic(['list', '--status', 'open'], fixture.context)).toBe(
			EXIT.ok,
		);
		expect(listTopics).toHaveBeenCalledWith('open');
		expect(fixture.stdoutJson()).toEqual({ topics: [TOPIC] });
	});

	it('closes a topic named by key, optionally withdrawing its open messages', async () => {
		const listTopics = vi.fn<HubClient['listTopics']>(async () =>
			succeed([TOPIC]),
		);
		const closeTopic = vi.fn<HubClient['closeTopic']>(async () =>
			succeed({ ...TOPIC, status: 'closed' }),
		);
		const fixture = createCommandFixture(
			stubHubClient({ listTopics, closeTopic }),
		);
		const code = await runTopic(
			['close', 'release-1.4', '--rescind-open', '--reason', 'obsolete'],
			fixture.context,
		);
		expect(code).toBe(EXIT.ok);
		expect(closeTopic).toHaveBeenCalledWith(TOPIC_ID, {
			rescindOpenMessages: true,
			reason: 'obsolete',
		});
	});

	it('prints a clear message with the count when the hub refuses to close', async () => {
		const closeTopic = vi.fn<HubClient['closeTopic']>(async () =>
			fail({
				kind: 'http' as const,
				status: 409,
				code: 'has_open_messages',
				message: 'still open',
				pointer: undefined,
				count: 2,
			}),
		);
		const fixture = createCommandFixture(stubHubClient({ closeTopic }));
		expect(await runTopic(['close', TOPIC_ID], fixture.context)).toBe(
			EXIT.failed,
		);
		expect(fixture.stderr()).toContain('topic was not closed');
		expect(fixture.stderr()).toContain('2 open messages');
		expect(fixture.stderr()).toContain('--rescind-open');
	});

	it('says the General topic cannot be closed', async () => {
		const closeTopic = async () =>
			fail({
				kind: 'http' as const,
				status: 409,
				code: 'general_topic_cannot_close',
				message: 'no',
				pointer: undefined,
			});
		const fixture = createCommandFixture(stubHubClient({ closeTopic }));
		expect(await runTopic(['close', TOPIC_ID], fixture.context)).toBe(
			EXIT.failed,
		);
		expect(fixture.stderr()).toContain("General topic can't be closed");
	});

	it('closes a topic named by id without looking it up', async () => {
		const closeTopic = vi.fn<HubClient['closeTopic']>(async () =>
			succeed(TOPIC),
		);
		await runTopic(
			['close', TOPIC_ID],
			createCommandFixture(stubHubClient({ closeTopic })).context,
		);
		expect(closeTopic).toHaveBeenCalledWith(TOPIC_ID, {
			rescindOpenMessages: undefined,
			reason: undefined,
		});
	});

	it('says so when no topic has the key', async () => {
		const fixture = createCommandFixture(
			stubHubClient({ listTopics: async () => succeed([]) }),
		);
		expect(await runTopic(['reopen', 'nothing'], fixture.context)).toBe(
			EXIT.usage,
		);
		expect(fixture.stderr()).toContain('no topic has the key "nothing"');
	});

	it.each([
		[[]],
		[['create']],
		[['create', 'k']],
		[['close']],
		[['close', 'k', '--reason', 'obsolete']],
		[['list', '--status', 'x']],
	])('rejects %j', async (args) => {
		expect(
			await runTopic(args, createCommandFixture(stubHubClient()).context),
		).toBe(EXIT.usage);
	});
});

describe('gentbox thread', () => {
	it('creates a thread under an inline topic', async () => {
		const createThread = vi.fn<HubClient['createThread']>(async () =>
			succeed({ wasCreated: true, record: THREAD }),
		);
		const code = await runThread(
			[
				'create',
				'phase-3',
				'--title',
				'Phase 3',
				'--topic',
				'release-1.4',
				'--topic-title',
				'Release 1.4',
			],
			createCommandFixture(stubHubClient({ createThread })).context,
		);
		expect(code).toBe(EXIT.ok);
		expect(createThread).toHaveBeenCalledWith({
			topic: { key: 'release-1.4', title: 'Release 1.4' },
			key: 'phase-3',
			title: 'Phase 3',
			summary: undefined,
			kind: undefined,
			conversation: { id: 'session-1' },
		});
	});

	it('--this-conversation makes the thread stand for the current conversation', async () => {
		const createThread = vi.fn<HubClient['createThread']>(async () =>
			succeed({ wasCreated: true, record: THREAD }),
		);
		await runThread(
			[
				'create',
				'session:web',
				'--title',
				'Release notes page',
				'--this-conversation',
			],
			createCommandFixture(stubHubClient({ createThread })).context,
		);
		expect(createThread.mock.calls[0]?.[0]).toMatchObject({
			kind: 'conversation',
			conversation: { id: 'session-1' },
		});
	});

	it('lists, closes, reopens and moves', async () => {
		const listTopics = vi.fn<HubClient['listTopics']>(async () =>
			succeed([TOPIC]),
		);
		const listThreads = vi.fn<HubClient['listThreads']>(async () =>
			succeed([THREAD]),
		);
		const closeThread = vi.fn<HubClient['closeThread']>(async () =>
			succeed(THREAD),
		);
		const reopenThread = vi.fn<HubClient['reopenThread']>(async () =>
			succeed(THREAD),
		);
		const moveThread = vi.fn<HubClient['moveThread']>(async () =>
			succeed(THREAD),
		);
		const context = createCommandFixture(
			stubHubClient({
				listTopics,
				listThreads,
				closeThread,
				reopenThread,
				moveThread,
			}),
		).context;
		expect(
			await runThread(
				['list', '--topic', 'release-1.4', '--status', 'closed'],
				context,
			),
		).toBe(EXIT.ok);
		expect(listThreads).toHaveBeenCalledWith(TOPIC_ID, 'closed');
		expect(
			await runThread(['close', THREAD_ID, '--rescind-open'], context),
		).toBe(EXIT.ok);
		expect(closeThread).toHaveBeenCalledWith(THREAD_ID, {
			rescindOpenMessages: true,
			reason: undefined,
		});
		const refused = createCommandFixture(
			stubHubClient({
				closeThread: async () =>
					fail({
						kind: 'http' as const,
						status: 409,
						code: 'has_open_messages',
						message: 'still open',
						pointer: undefined,
						count: 1,
					}),
			}),
		);
		expect(await runThread(['close', THREAD_ID], refused.context)).toBe(
			EXIT.failed,
		);
		expect(refused.stderr()).toContain('thread was not closed');
		expect(refused.stderr()).toContain('1 open message ');
		expect(await runThread(['reopen', THREAD_ID], context)).toBe(EXIT.ok);
		expect(
			await runThread(['move', THREAD_ID, '--topic', TOPIC_ID], context),
		).toBe(EXIT.ok);
		expect(moveThread).toHaveBeenCalledWith(THREAD_ID, {
			topic: { id: TOPIC_ID },
		});
	});

	it.each([
		[[]],
		[['create']],
		[['create', 'k']],
		[['move', THREAD_ID]],
		[['close']],
		[['bogus']],
	])('rejects %j', async (args) => {
		expect(
			await runThread(args, createCommandFixture(stubHubClient()).context),
		).toBe(EXIT.usage);
	});
});
