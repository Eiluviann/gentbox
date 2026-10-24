import { createHmac, randomBytes, timingSafeEqual } from 'node:crypto';
import type { z } from 'zod';
import {
	type DiscoveryEnvironment,
	discoverHubEndpoints,
	type HubEndpoint,
} from './discovery.ts';
import {
	type HttpTransport,
	type HubTarget,
	type RawRequest,
	sendHttpRequest,
} from './http-transport.ts';
import {
	type ActionResponse,
	actionResponseSchema,
	type CloseRequest,
	errorBodySchema,
	helloResponseSchema,
	type IngestResponse,
	ingestResponseSchema,
	type MessageV1,
	type RescindReason,
	type RescindRequest,
	rescindedIdsSchema,
	type ThreadCreate,
	type ThreadMove,
	type ThreadRecord,
	type TopicCreate,
	type TopicRecord,
	threadListSchema,
	threadRecordSchema,
	topicListSchema,
	topicRecordSchema,
} from './message-types.ts';
import { fail, type Result, succeed } from './result.ts';

/** Why a call to the hub did not give a usable answer. */
export type HubFailure =
	| {
			kind: 'http';
			status: number;
			/** The hub's stable error code (`schema_invalid`, `not_owner`, `already_closed`, …). */
			code: string;
			message: string;
			/** JSON pointer of the failing field, for `schema_invalid`. */
			pointer: string | undefined;
	  }
	| { kind: 'transport'; message: string }
	| { kind: 'protocol'; message: string };

/** The self-asserted identity sent in `Gentbox-Agent` and `Gentbox-Conversation` (architecture §8). */
export type HubIdentity = {
	agent: string;
	conversationId: string | undefined;
};

export type Created<T> = { wasCreated: boolean; record: T };

type GroupStatus = 'open' | 'closed';

/** The routes a sender can use. None answers or lists messages: the sender token cannot. */
export type HubClient = {
	postMessage(
		message: MessageV1,
	): Promise<Result<Created<IngestResponse>, HubFailure>>;
	/** Holds up to `waitSeconds` (the hub caps it at 55) and answers `pending` at the deadline. */
	getResponse(
		messageId: string,
		waitSeconds: number,
		signal?: AbortSignal,
	): Promise<Result<ActionResponse, HubFailure>>;
	rescindMessage(
		messageId: string,
		reason: RescindReason,
	): Promise<Result<ActionResponse, HubFailure>>;
	rescindByCorrelationKeys(
		request: RescindRequest,
	): Promise<Result<string[], HubFailure>>;
	reportProgress(): Promise<Result<string[], HubFailure>>;
	setFocus(isFocused: boolean): Promise<Result<undefined, HubFailure>>;
	createTopic(
		request: TopicCreate,
	): Promise<Result<Created<TopicRecord>, HubFailure>>;
	listTopics(
		status: GroupStatus | undefined,
	): Promise<Result<TopicRecord[], HubFailure>>;
	closeTopic(
		topicId: string,
		request: CloseRequest,
	): Promise<Result<TopicRecord, HubFailure>>;
	reopenTopic(topicId: string): Promise<Result<TopicRecord, HubFailure>>;
	createThread(
		request: ThreadCreate,
	): Promise<Result<Created<ThreadRecord>, HubFailure>>;
	listThreads(
		topicId: string | undefined,
		status: GroupStatus | undefined,
	): Promise<Result<ThreadRecord[], HubFailure>>;
	closeThread(
		threadId: string,
		request: CloseRequest,
	): Promise<Result<ThreadRecord, HubFailure>>;
	reopenThread(threadId: string): Promise<Result<ThreadRecord, HubFailure>>;
	moveThread(
		threadId: string,
		request: ThreadMove,
	): Promise<Result<ThreadRecord, HubFailure>>;
};

export type HubSession = {
	target: HubTarget;
	/** Sent as the bearer token; callers must have verified the hub first when the target is TCP. */
	token: string;
	identity: HubIdentity;
	transport: HttpTransport;
	sleep: (milliseconds: number) => Promise<void>;
};

const REQUEST_TIMEOUT_MILLISECONDS = 15_000;
const LONG_POLL_MARGIN_MILLISECONDS = 15_000;
const MAX_RATE_LIMIT_RETRIES = 2;
const MAX_RETRY_AFTER_SECONDS = 5;
const HELLO_DOMAIN = 'gentbox-hello-v1\0';

/** What `hello` signs: `HMAC-SHA256(token, "gentbox-hello-v1\0" + port + "\0" + nonce)`, base64url. */
export function computeHelloProof(
	token: string,
	port: number,
	nonce: string,
): string {
	return createHmac('sha256', token)
		.update(`${HELLO_DOMAIN}${port}\0${nonce}`)
		.digest('base64url');
}

/** Whether `proof` is the hub's answer for this token, port and nonce, compared in constant time. */
export function isValidHelloProof(
	token: string,
	port: number,
	nonce: string,
	proof: string,
): boolean {
	const expected = Buffer.from(
		computeHelloProof(token, port, nonce),
		'base64url',
	);
	const presented = Buffer.from(proof, 'base64url');
	return (
		presented.length === expected.length && timingSafeEqual(presented, expected)
	);
}

export function describeHubFailure(failure: HubFailure): string {
	switch (failure.kind) {
		case 'http': {
			const where =
				failure.pointer === undefined ? '' : ` at ${failure.pointer}`;
			return `${failure.code}${where}: ${failure.message}`;
		}
		case 'transport':
			return `cannot reach the hub: ${failure.message}`;
		case 'protocol':
			return `unexpected answer from the hub: ${failure.message}`;
	}
}

export type ConnectOptions = {
	environment: DiscoveryEnvironment;
	identity: HubIdentity;
	transport?: HttpTransport;
	sleep?: (milliseconds: number) => Promise<void>;
};

/**
 * Finds the running hub and returns a client for it. Per endpoint the Unix
 * socket comes first (only this user can reach it); TCP is used only after the
 * hub proves it holds the token, and the token is not sent before that.
 */
export async function connectToHub(
	options: ConnectOptions,
): Promise<Result<HubClient, string>> {
	const transport = options.transport ?? sendHttpRequest;
	const sleep = options.sleep ?? defaultSleep;
	const { endpoints, skipped } = discoverHubEndpoints(options.environment);
	const problems = [...skipped];
	for (const endpoint of endpoints) {
		const target = await reachEndpoint(endpoint, transport);
		if (target.ok) {
			const session = {
				target: target.value,
				token: endpoint.token,
				identity: options.identity,
				transport,
				sleep,
			};
			return succeed(createHubClient(session));
		}
		problems.push(`${endpoint.directory}: ${target.error}`);
	}
	return fail(
		[
			'Gentbox is not running or cannot be reached. Start the Gentbox app (or gentboxd).',
			...problems.map((problem) => `  - ${problem}`),
		].join('\n'),
	);
}

async function reachEndpoint(
	endpoint: HubEndpoint,
	transport: HttpTransport,
): Promise<Result<HubTarget, string>> {
	const problems: string[] = [];
	if (endpoint.socketPath !== undefined) {
		const target: HubTarget = {
			kind: 'socket',
			socketPath: endpoint.socketPath,
		};
		const hello = await sayHello(transport, target, randomNonce());
		if (hello.ok) return succeed(target);
		problems.push(`socket: ${hello.error}`);
	}
	if (endpoint.port !== undefined) {
		const target: HubTarget = { kind: 'tcp', port: endpoint.port };
		const proven = await proveHubOverTcp(
			transport,
			endpoint.token,
			endpoint.port,
		);
		if (proven.ok) return succeed(target);
		problems.push(`port ${endpoint.port}: ${proven.error}`);
	}
	if (problems.length === 0)
		problems.push('no socket and no port: the hub is not serving');
	return fail(problems.join('; '));
}

async function proveHubOverTcp(
	transport: HttpTransport,
	token: string,
	port: number,
): Promise<Result<undefined, string>> {
	const nonce = randomNonce();
	const hello = await sayHello(transport, { kind: 'tcp', port }, nonce);
	if (!hello.ok) return hello;
	if (!isValidHelloProof(token, port, nonce, hello.value)) {
		return fail("the answer is not the hub's proof; the token was not sent");
	}
	return succeed(undefined);
}

/** `POST /v1/hello`, the one request without the token. Returns the hub's proof. */
async function sayHello(
	transport: HttpTransport,
	target: HubTarget,
	nonce: string,
): Promise<Result<string, string>> {
	const response = await transport(target, {
		method: 'POST',
		path: '/v1/hello',
		headers: { accept: 'application/json' },
		body: JSON.stringify({ nonce }),
		timeoutMilliseconds: 5_000,
		signal: undefined,
	});
	if (!response.ok) return fail(response.error);
	if (response.value.status !== 200)
		return fail(`hello answered ${response.value.status}`);
	const parsed = parseJson(response.value.body, helloResponseSchema);
	if (!parsed.ok) return fail(parsed.error);
	return succeed(parsed.value.proof);
}

export function createHubClient(session: HubSession): HubClient {
	const { agent, conversationId } = session.identity;
	const conversationPath =
		conversationId === undefined
			? undefined
			: `/v1/conversations/${agent}/${conversationId}`;
	return {
		postMessage: async (message) => {
			const result = await callJson(
				session,
				{ method: 'POST', path: '/v1/messages', body: message },
				ingestResponseSchema,
			);
			return mapResult(result, (answer) => ({
				wasCreated: answer.status === 201,
				record: answer.value,
			}));
		},
		getResponse: (messageId, waitSeconds, signal) =>
			callValue(
				session,
				{
					method: 'GET',
					path: `/v1/messages/${encodeURIComponent(messageId)}/response?wait=${Math.max(0, Math.floor(waitSeconds))}`,
					timeoutMilliseconds:
						waitSeconds * 1000 + LONG_POLL_MARGIN_MILLISECONDS,
					signal,
				},
				actionResponseSchema,
			),
		rescindMessage: (messageId, reason) =>
			callValue(
				session,
				{
					method: 'DELETE',
					path: `/v1/messages/${encodeURIComponent(messageId)}`,
					body: { reason },
				},
				actionResponseSchema,
			),
		rescindByCorrelationKeys: async (request) => {
			if (conversationPath === undefined) return fail(needsConversation());
			const result = await callValue(
				session,
				{ method: 'POST', path: `${conversationPath}/rescind`, body: request },
				rescindedIdsSchema,
			);
			return mapResult(result, (answer) => answer.rescinded);
		},
		reportProgress: async () => {
			if (conversationPath === undefined) return fail(needsConversation());
			const result = await callValue(
				session,
				{ method: 'POST', path: `${conversationPath}/progress`, body: {} },
				rescindedIdsSchema,
			);
			return mapResult(result, (answer) => answer.rescinded);
		},
		setFocus: async (isFocused) => {
			const exchanged = await exchange(session, {
				method: 'POST',
				path: '/v1/presence/focus',
				body: { focused: isFocused },
			});
			return mapResult(exchanged, () => undefined);
		},
		createTopic: async (request) => {
			const result = await callJson(
				session,
				{ method: 'POST', path: '/v1/topics', body: request },
				topicRecordSchema,
			);
			return mapResult(result, (answer) => ({
				wasCreated: answer.status === 201,
				record: answer.value,
			}));
		},
		listTopics: async (status) => {
			const query = status === undefined ? '' : `?status=${status}`;
			const result = await callValue(
				session,
				{ method: 'GET', path: `/v1/topics${query}` },
				topicListSchema,
			);
			return mapResult(result, (answer) => answer.topics);
		},
		closeTopic: (topicId, request) =>
			callValue(
				session,
				{
					method: 'POST',
					path: `/v1/topics/${encodeURIComponent(topicId)}/close`,
					body: request,
				},
				topicRecordSchema,
			),
		reopenTopic: (topicId) =>
			callValue(
				session,
				{
					method: 'POST',
					path: `/v1/topics/${encodeURIComponent(topicId)}/reopen`,
					body: {},
				},
				topicRecordSchema,
			),
		createThread: async (request) => {
			const result = await callJson(
				session,
				{ method: 'POST', path: '/v1/threads', body: request },
				threadRecordSchema,
			);
			return mapResult(result, (answer) => ({
				wasCreated: answer.status === 201,
				record: answer.value,
			}));
		},
		listThreads: async (topicId, status) => {
			const query = new URLSearchParams();
			if (topicId !== undefined) query.set('topic', topicId);
			if (status !== undefined) query.set('status', status);
			const suffix = query.size === 0 ? '' : `?${query.toString()}`;
			const result = await callValue(
				session,
				{ method: 'GET', path: `/v1/threads${suffix}` },
				threadListSchema,
			);
			return mapResult(result, (answer) => answer.threads);
		},
		closeThread: (threadId, request) =>
			callValue(
				session,
				{
					method: 'POST',
					path: `/v1/threads/${encodeURIComponent(threadId)}/close`,
					body: request,
				},
				threadRecordSchema,
			),
		reopenThread: (threadId) =>
			callValue(
				session,
				{
					method: 'POST',
					path: `/v1/threads/${encodeURIComponent(threadId)}/reopen`,
					body: {},
				},
				threadRecordSchema,
			),
		moveThread: (threadId, request) =>
			callValue(
				session,
				{
					method: 'PUT',
					path: `/v1/threads/${encodeURIComponent(threadId)}/topic`,
					body: request,
				},
				threadRecordSchema,
			),
	};
}

type Call = {
	method: RawRequest['method'];
	path: string;
	body?: unknown;
	timeoutMilliseconds?: number;
	signal?: AbortSignal | undefined;
};

type Answer<T> = { status: number; value: T };

async function callValue<T>(
	session: HubSession,
	call: Call,
	schema: z.ZodType<T>,
): Promise<Result<T, HubFailure>> {
	return mapResult(
		await callJson(session, call, schema),
		(answer) => answer.value,
	);
}

async function callJson<T>(
	session: HubSession,
	call: Call,
	schema: z.ZodType<T>,
): Promise<Result<Answer<T>, HubFailure>> {
	const exchanged = await exchange(session, call);
	if (!exchanged.ok) return exchanged;
	const parsed = parseJson(exchanged.value.body, schema);
	if (!parsed.ok) return fail({ kind: 'protocol', message: parsed.error });
	return succeed({ status: exchanged.value.status, value: parsed.value });
}

/** One authenticated call. Retries a 429 (rejected before any work) a bounded number of times. */
async function exchange(
	session: HubSession,
	call: Call,
): Promise<Result<{ status: number; body: string }, HubFailure>> {
	const request: RawRequest = {
		method: call.method,
		path: call.path,
		headers: authenticatedHeaders(session),
		body: call.body === undefined ? undefined : JSON.stringify(call.body),
		timeoutMilliseconds:
			call.timeoutMilliseconds ?? REQUEST_TIMEOUT_MILLISECONDS,
		signal: call.signal,
	};
	for (let attempt = 0; ; attempt++) {
		const response = await session.transport(session.target, request);
		if (!response.ok)
			return fail({ kind: 'transport', message: response.error });
		const { status, headers, body } = response.value;
		if (status >= 200 && status < 300) return succeed({ status, body });
		const isRateLimited = status === 429 && attempt < MAX_RATE_LIMIT_RETRIES;
		if (!isRateLimited) return fail(httpFailure(status, body));
		await session.sleep(retryAfterMilliseconds(headers['retry-after']));
	}
}

function authenticatedHeaders(session: HubSession): Record<string, string> {
	const headers: Record<string, string> = {
		accept: 'application/json',
		authorization: `Bearer ${session.token}`,
		'gentbox-agent': session.identity.agent,
	};
	if (session.identity.conversationId !== undefined) {
		headers['gentbox-conversation'] = session.identity.conversationId;
	}
	return headers;
}

function httpFailure(status: number, body: string): HubFailure {
	const parsed = parseJson(body, errorBodySchema);
	if (!parsed.ok)
		return {
			kind: 'http',
			status,
			code: `http_${status}`,
			message: body.slice(0, 200),
			pointer: undefined,
		};
	return {
		kind: 'http',
		status,
		code: parsed.value.code,
		message: parsed.value.message,
		pointer: parsed.value.pointer,
	};
}

function retryAfterMilliseconds(header: string | string[] | undefined): number {
	const seconds = Number(Array.isArray(header) ? header[0] : header);
	const bounded =
		Number.isFinite(seconds) && seconds > 0
			? Math.min(seconds, MAX_RETRY_AFTER_SECONDS)
			: 1;
	return bounded * 1000;
}

function parseJson<T>(text: string, schema: z.ZodType<T>): Result<T, string> {
	let json: unknown;
	try {
		json = JSON.parse(text);
	} catch {
		return fail('the body is not JSON');
	}
	const parsed = schema.safeParse(json);
	if (!parsed.success) {
		const issue = parsed.error.issues[0];
		return fail(
			`the body does not match the schema (${issue?.path.join('/') ?? ''}: ${issue?.message ?? 'invalid'})`,
		);
	}
	return succeed(parsed.data);
}

function mapResult<T, U, E>(
	result: Result<T, E>,
	transform: (value: T) => U,
): Result<U, E> {
	return result.ok ? succeed(transform(result.value)) : result;
}

function needsConversation(): HubFailure {
	return {
		kind: 'protocol',
		message:
			'this call needs a conversation id (GENTBOX_CONVERSATION or --conversation)',
	};
}

function randomNonce(): string {
	return randomBytes(16).toString('base64url');
}

function defaultSleep(milliseconds: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
