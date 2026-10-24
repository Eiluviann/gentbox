import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
	computeHelloProof,
	connectToHub,
	describeHubFailure,
	type HubClient,
	isValidHelloProof,
} from './hub-client.ts';
import type { MessageV1 } from './message-types.ts';
import {
	type FakeAnswer,
	type FakeHub,
	startFakeHub,
} from './test-support/fake-hub.ts';

const TOKEN = 'unit-test-token-'.padEnd(43, '0');
const MESSAGE_ID = '01JBMA2W8R6T4Y2V0P8N6M4K2H';
const TOPIC_ID = '01JBM6R3K9P2W5X8Z1C4V7N0QD';
const THREAD_ID = '01JBM9P3H4S5E6J7N8G9E0S1TQ';
const IDENTITY = { agent: 'claude-code', conversationId: 'conversation-1' };

const INGEST_RESPONSE = {
	id: MESSAGE_ID,
	type: 'decision_required',
	urgent: false,
	blocking: 'blocking',
	topicId: TOPIC_ID,
	threadId: THREAD_ID,
	state: 'open',
	shouldWait: true,
	warnings: [],
};

const MESSAGE: MessageV1 = {
	version: 1,
	source: { agent: 'claude-code' },
	conversation: { id: 'conversation-1' },
	title: 'Allow Bash to run the tests?',
};

let directory: string;
let hub: FakeHub | undefined;

beforeEach(() => {
	directory = mkdtempSync(join(tmpdir(), 'gentbox-client-'));
});

afterEach(async () => {
	await hub?.close();
	hub = undefined;
	rmSync(directory, { recursive: true, force: true });
});

async function connect(
	handler: (url: string, method: string) => FakeAnswer,
	transports: ('socket' | 'tcp')[] = ['socket', 'tcp'],
	isImpostor = false,
): Promise<{ client: HubClient; hub: FakeHub }> {
	hub = await startFakeHub({
		directory,
		token: TOKEN,
		transports,
		isImpostor,
		handler: (request) => handler(request.url, request.method),
	});
	const connected = await connectToHub({
		environment: hub.environment,
		identity: IDENTITY,
		sleep: () => Promise.resolve(),
	});
	if (!connected.ok) throw new Error(connected.error);
	return { client: connected.value, hub };
}

describe('the hello proof', () => {
	it('is the HMAC over the domain, the port and the nonce, and binds the port', () => {
		const proof = computeHelloProof(TOKEN, 4242, 'a-nonce');
		expect(isValidHelloProof(TOKEN, 4242, 'a-nonce', proof)).toBe(true);
		expect(isValidHelloProof(TOKEN, 4243, 'a-nonce', proof)).toBe(false);
		expect(isValidHelloProof(TOKEN, 4242, 'another-nonce', proof)).toBe(false);
		expect(isValidHelloProof(`${TOKEN}x`, 4242, 'a-nonce', proof)).toBe(false);
		expect(isValidHelloProof(TOKEN, 4242, 'a-nonce', 'not base64!')).toBe(
			false,
		);
		expect(isValidHelloProof(TOKEN, 4242, 'a-nonce', '')).toBe(false);
	});

	it('matches an independent HMAC (openssl, key Jefe, port 4242)', () => {
		// base64url(HMAC-SHA256("Jefe", "gentbox-hello-v1\0" + "4242" + "\0" + "what do ya want for nothing?"))
		expect(
			computeHelloProof('Jefe', 4242, 'what do ya want for nothing?'),
		).toBe('eVOSF5BqQGu8g3L7B2lQ0cMBYKJNNNwTH9NWbqLk4nk');
	});
});

describe('connectToHub', () => {
	it('prefers the socket, and never says hello over TCP when the socket works', async () => {
		const connection = await connect(() => ({
			status: 201,
			body: INGEST_RESPONSE,
		}));
		const posted = await connection.client.postMessage(MESSAGE);
		expect(posted).toMatchObject({ ok: true, value: { wasCreated: true } });
		expect(
			connection.hub.requests.every(
				(request) => request.transport === 'socket',
			),
		).toBe(true);
		const authorized = connection.hub.requests.find(
			(request) => request.url === '/v1/messages',
		);
		expect(authorized?.headers.authorization).toBe(`Bearer ${TOKEN}`);
	});

	it('falls back to TCP after the hub proves itself, and only then sends the token', async () => {
		const connection = await connect(
			() => ({ status: 201, body: INGEST_RESPONSE }),
			['tcp'],
		);
		const posted = await connection.client.postMessage(MESSAGE);
		expect(posted.ok).toBe(true);
		// One proof when connecting, one more right before the request that carries the token.
		const [firstHello, secondHello, post] = connection.hub.requests;
		expect([firstHello?.url, secondHello?.url, post?.url]).toEqual([
			'/v1/hello',
			'/v1/hello',
			'/v1/messages',
		]);
		expect(firstHello?.headers.authorization).toBeUndefined();
		expect(secondHello?.headers.authorization).toBeUndefined();
		expect(post?.headers.authorization).toBe(`Bearer ${TOKEN}`);
	});

	it('proves the hub again before every request over TCP: a squatter on a stale port never gets the token', async () => {
		const onConnectionLost = vi.fn();
		hub = await startFakeHub({
			directory,
			token: TOKEN,
			transports: ['tcp'],
			handler: () => ({ status: 201, body: INGEST_RESPONSE }),
		});
		const connected = await connectToHub({
			environment: hub.environment,
			identity: IDENTITY,
			sleep: () => Promise.resolve(),
			onConnectionLost,
		});
		if (!connected.ok) throw new Error(connected.error);
		expect((await connected.value.postMessage(MESSAGE)).ok).toBe(true);
		const requestsBeforeTakeover = hub.requests.length;

		hub.becomeImpostor();
		const posted = await connected.value.postMessage(MESSAGE);

		expect(posted).toMatchObject({ ok: false, error: { kind: 'protocol' } });
		if (!posted.ok)
			expect(describeHubFailure(posted.error)).toContain('token was not sent');
		const afterTakeover = hub.requests.slice(requestsBeforeTakeover);
		expect(afterTakeover.map((request) => request.url)).toEqual(['/v1/hello']);
		expect(
			afterTakeover.some(
				(request) => request.headers.authorization !== undefined,
			),
		).toBe(false);
		expect(JSON.stringify(afterTakeover)).not.toContain(TOKEN);
		expect(onConnectionLost).toHaveBeenCalledOnce();
	});

	it('does not prove the hub again over the socket', async () => {
		const connection = await connect(
			() => ({ status: 201, body: INGEST_RESPONSE }),
			['socket', 'tcp'],
		);
		await connection.client.postMessage(MESSAGE);
		await connection.client.postMessage(MESSAGE);
		expect(
			connection.hub.requests.filter((request) => request.url === '/v1/hello'),
		).toHaveLength(1);
	});

	it('reports a lost connection so the caller can rediscover the hub', async () => {
		const onConnectionLost = vi.fn();
		hub = await startFakeHub({
			directory,
			token: TOKEN,
			transports: ['socket'],
			handler: () => ({ status: 201, body: INGEST_RESPONSE }),
		});
		const connected = await connectToHub({
			environment: hub.environment,
			identity: IDENTITY,
			onConnectionLost,
		});
		if (!connected.ok) throw new Error(connected.error);
		await hub.close();
		hub = undefined;
		expect(await connected.value.postMessage(MESSAGE)).toMatchObject({
			ok: false,
			error: { kind: 'transport' },
		});
		expect(onConnectionLost).toHaveBeenCalledOnce();
	});

	it('does not report a lost connection for an HTTP error from a hub that is there', async () => {
		const onConnectionLost = vi.fn();
		hub = await startFakeHub({
			directory,
			token: TOKEN,
			transports: ['socket'],
			handler: () => ({
				status: 422,
				body: { code: 'schema_invalid', message: 'bad', pointer: '/title' },
			}),
		});
		const connected = await connectToHub({
			environment: hub.environment,
			identity: IDENTITY,
			onConnectionLost,
		});
		if (!connected.ok) throw new Error(connected.error);
		await connected.value.postMessage(MESSAGE);
		expect(onConnectionLost).not.toHaveBeenCalled();
	});

	it('never sends the token to a hub whose proof is wrong', async () => {
		hub = await startFakeHub({
			directory,
			token: TOKEN,
			transports: ['tcp'],
			isImpostor: true,
			handler: () => ({ status: 201, body: INGEST_RESPONSE }),
		});
		const connected = await connectToHub({
			environment: hub.environment,
			identity: IDENTITY,
		});
		expect(connected.ok).toBe(false);
		if (!connected.ok) expect(connected.error).toContain('token was not sent');
		expect(hub.requests).toHaveLength(1);
		expect(hub.requests.every((request) => request.url === '/v1/hello')).toBe(
			true,
		);
		expect(
			hub.requests.some(
				(request) => request.headers.authorization !== undefined,
			),
		).toBe(false);
		expect(JSON.stringify(hub.requests)).not.toContain(TOKEN);
	});

	it('says how to start the hub when nothing is running', async () => {
		const connected = await connectToHub({
			environment: {
				apiDirectoryOverride: join(directory, 'API'),
				homeDirectory: directory,
				xdgStateHome: undefined,
			},
			identity: IDENTITY,
		});
		expect(connected).toMatchObject({ ok: false });
		if (!connected.ok)
			expect(connected.error).toContain('Start the Gentbox app');
	});
});

describe('request building', () => {
	it('posts a message with the self-asserted identity headers', async () => {
		const connection = await connect(() => ({
			status: 201,
			body: INGEST_RESPONSE,
		}));
		await connection.client.postMessage(MESSAGE);
		const request = connection.hub.requests.find(
			(recorded) => recorded.url === '/v1/messages',
		);
		expect(request?.method).toBe('POST');
		expect(request?.headers['gentbox-agent']).toBe('claude-code');
		expect(request?.headers['gentbox-conversation']).toBe('conversation-1');
		expect(JSON.parse(request?.body ?? '')).toEqual(MESSAGE);
	});

	it('long-polls with the wait in whole seconds and parses the answer', async () => {
		const connection = await connect(() => ({
			status: 200,
			body: {
				messageId: MESSAGE_ID,
				status: 'answered',
				actionId: 'allow',
				actionKind: 'approve',
			},
		}));
		const answer = await connection.client.getResponse(MESSAGE_ID, 20.7);
		expect(answer).toMatchObject({
			ok: true,
			value: { status: 'answered', actionKind: 'approve' },
		});
		expect(connection.hub.requests.at(-1)?.url).toBe(
			`/v1/messages/${MESSAGE_ID}/response?wait=20`,
		);
	});

	it('rescinds one message with DELETE and a reason', async () => {
		const connection = await connect(() => ({
			status: 200,
			body: {
				messageId: MESSAGE_ID,
				status: 'rescinded',
				rescindReason: 'obsolete',
			},
		}));
		await connection.client.rescindMessage(MESSAGE_ID, 'obsolete');
		const request = connection.hub.requests.at(-1);
		expect([request?.method, request?.url]).toEqual([
			'DELETE',
			`/v1/messages/${MESSAGE_ID}`,
		]);
		expect(JSON.parse(request?.body ?? '')).toEqual({ reason: 'obsolete' });
	});

	it('rescinds by correlation keys and reports progress on the conversation route', async () => {
		const connection = await connect(() => ({
			status: 200,
			body: { rescinded: [MESSAGE_ID] },
		}));
		const rescinded = await connection.client.rescindByCorrelationKeys({
			reason: 'superseded',
			correlationKeys: ['plan'],
		});
		expect(rescinded).toEqual({ ok: true, value: [MESSAGE_ID] });
		await connection.client.reportProgress();
		const urls = connection.hub.requests.map((request) => request.url);
		expect(urls).toContain(
			'/v1/conversations/claude-code/conversation-1/rescind',
		);
		expect(urls).toContain(
			'/v1/conversations/claude-code/conversation-1/progress',
		);
	});

	it('refuses a conversation route without a conversation id', async () => {
		hub = await startFakeHub({
			directory,
			token: TOKEN,
			transports: ['socket'],
			handler: () => ({ status: 200, body: {} }),
		});
		const connected = await connectToHub({
			environment: hub.environment,
			identity: { agent: 'cli', conversationId: undefined },
		});
		if (!connected.ok) throw new Error(connected.error);
		const progress = await connected.value.reportProgress();
		expect(progress).toMatchObject({ ok: false, error: { kind: 'protocol' } });
	});

	it('sends focus as a boolean body and accepts 204', async () => {
		const connection = await connect(() => ({ status: 204 }));
		expect(await connection.client.setFocus(true)).toEqual({
			ok: true,
			value: undefined,
		});
		expect(JSON.parse(connection.hub.requests.at(-1)?.body ?? '')).toEqual({
			focused: true,
		});
	});

	it('builds the topic and thread routes', async () => {
		const topic = {
			id: TOPIC_ID,
			title: 'Release 1.4',
			implicit: false,
			status: 'open',
			createdBy: { agent: 'claude-code' },
			createdAt: '2026-10-23T18:30:00Z',
			updatedAt: '2026-10-23T18:30:00Z',
		};
		const connection = await connect((url) => {
			if (url.startsWith('/v1/topics') && url.includes('?'))
				return { status: 200, body: { topics: [topic] } };
			if (url.startsWith('/v1/threads?'))
				return { status: 200, body: { threads: [] } };
			return { status: 201, body: topic };
		});
		const created = await connection.client.createTopic({
			key: 'release-1.4',
			title: 'Release 1.4',
		});
		expect(created).toMatchObject({ ok: true, value: { wasCreated: true } });
		await connection.client.listTopics('open');
		await connection.client.listThreads(TOPIC_ID, 'closed');
		await connection.client.closeTopic(TOPIC_ID, { rescindOpenMessages: true });
		const urls = connection.hub.requests.map(
			(request) => `${request.method} ${request.url}`,
		);
		expect(urls).toContain('POST /v1/topics');
		expect(urls).toContain('GET /v1/topics?status=open');
		expect(urls).toContain(`GET /v1/threads?topic=${TOPIC_ID}&status=closed`);
		expect(urls).toContain(`POST /v1/topics/${TOPIC_ID}/close`);
	});
});

describe('a newer hub within schema v1', () => {
	const FutureWarning = {
		code: 'title_too_vague',
		field: 'title',
		message: 'Be more specific.',
		hint: 'new',
	};
	const topic = {
		id: TOPIC_ID,
		title: 'Release 1.4',
		implicit: false,
		status: 'open',
		createdBy: { agent: 'claude-code', host: 'new-field' },
		createdAt: '2026-10-23T18:30:00Z',
		updatedAt: '2026-10-23T18:30:00Z',
		pinned: true,
	};

	it('accepts a new warning code and a new field in the ingest response, so the message is not lost', async () => {
		const connection = await connect(() => ({
			status: 201,
			body: { ...INGEST_RESPONSE, priorityLane: 2, warnings: [FutureWarning] },
		}));
		const posted = await connection.client.postMessage(MESSAGE);
		expect(posted).toMatchObject({
			ok: true,
			value: { record: { id: MESSAGE_ID } },
		});
		expect(posted.ok && posted.value.record.warnings?.[0]?.code).toBe(
			'title_too_vague',
		);
	});

	it('accepts new fields in an action response, a rescind answer, topic records and lists', async () => {
		const connection = await connect((url) => {
			if (url.includes('/response')) {
				return {
					status: 200,
					body: {
						messageId: MESSAGE_ID,
						status: 'answered',
						actionKind: 'approve',
						reviewedBy: 'x',
					},
				};
			}
			if (url.startsWith('/v1/topics?'))
				return { status: 200, body: { topics: [topic], nextPage: null } };
			if (url.includes('/progress'))
				return { status: 200, body: { rescinded: [], count: 0 } };
			return { status: 201, body: topic };
		});
		expect((await connection.client.getResponse(MESSAGE_ID, 0)).ok).toBe(true);
		expect((await connection.client.listTopics('open')).ok).toBe(true);
		expect((await connection.client.reportProgress()).ok).toBe(true);
		expect(
			(await connection.client.createTopic({ key: 'k', title: 'T' })).ok,
		).toBe(true);
	});
});

describe('failures', () => {
	it('maps a hub error body to a failure that names the code and the JSON pointer', async () => {
		const connection = await connect(() => ({
			status: 422,
			body: {
				code: 'schema_invalid',
				message: 'title is too long',
				pointer: '/title',
			},
		}));
		const posted = await connection.client.postMessage(MESSAGE);
		expect(posted).toEqual({
			ok: false,
			error: {
				kind: 'http',
				status: 422,
				code: 'schema_invalid',
				message: 'title is too long',
				pointer: '/title',
			},
		});
		if (!posted.ok)
			expect(describeHubFailure(posted.error)).toBe(
				'schema_invalid at /title: title is too long',
			);
	});

	it('retries a 429 a bounded number of times', async () => {
		let calls = 0;
		const connection = await connect(() => {
			calls++;
			return calls < 3
				? {
						status: 429,
						headers: { 'retry-after': '1' },
						body: { code: 'rate_limited', message: 'slow down' },
					}
				: { status: 201, body: INGEST_RESPONSE };
		});
		expect((await connection.client.postMessage(MESSAGE)).ok).toBe(true);
		expect(calls).toBe(3);
		calls = -10;
		const never = await connection.client.postMessage(MESSAGE);
		expect(never).toMatchObject({ ok: false, error: { status: 429 } });
	});

	it('reports an answer that does not match the schema as a protocol failure', async () => {
		const connection = await connect(() => ({
			status: 201,
			body: { id: 'nope' },
		}));
		expect(await connection.client.postMessage(MESSAGE)).toMatchObject({
			ok: false,
			error: { kind: 'protocol' },
		});
	});
});
