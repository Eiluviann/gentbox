import { describe, expect, it, vi } from 'vitest';
import { askAndWait, waitForAnswer } from './ask-flow.ts';
import type { HubClient } from './hub-client.ts';
import { fail, succeed } from './result.ts';
import {
	actionResponse,
	ingestResponse,
	MESSAGE_ID,
	SAMPLE_MESSAGE,
} from './test-support/hub-fixtures.ts';
import { stubHubClient } from './test-support/stub-hub-client.ts';

/** A clock that moves forward by the seconds each long-poll was asked to hold. */
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

describe('waitForAnswer', () => {
	it('polls again after pending and returns the answer', async () => {
		const responses = [
			actionResponse('pending'),
			actionResponse('answered', {
				actionKind: 'approve',
				actionId: 'approve',
			}),
		];
		const getResponse = vi.fn<HubClient['getResponse']>(async () =>
			succeed(responses.shift() ?? actionResponse('pending')),
		);
		const outcome = await waitForAnswer(
			stubHubClient({ getResponse }),
			MESSAGE_ID,
			{ timeoutSeconds: 120 },
		);
		expect(outcome).toMatchObject({
			kind: 'answered',
			response: { actionKind: 'approve' },
		});
		expect(getResponse).toHaveBeenCalledTimes(2);
		expect(getResponse.mock.calls[0]?.[1]).toBe(50);
	});

	it('asks for no more than the time that is left', async () => {
		const clock = fakeClock();
		const getResponse = vi.fn<HubClient['getResponse']>(
			async (_id, seconds) => {
				clock.advance(seconds);
				return succeed(actionResponse('pending'));
			},
		);
		const outcome = await waitForAnswer(
			stubHubClient({ getResponse }),
			MESSAGE_ID,
			{ timeoutSeconds: 70, ...clock },
		);
		expect(outcome).toEqual({ kind: 'timed_out' });
		expect(getResponse.mock.calls.map((call) => call[1])).toEqual([50, 20]);
	});

	it('returns deferred as soon as the user defers', async () => {
		const getResponse = async () => succeed(actionResponse('deferred'));
		expect(
			await waitForAnswer(stubHubClient({ getResponse }), MESSAGE_ID, {
				timeoutSeconds: 60,
			}),
		).toMatchObject({
			kind: 'deferred',
		});
	});

	it.each(['dismissed', 'rescinded', 'expired', 'gone'] as const)(
		'ends on %s',
		async (status) => {
			const getResponse = async () => succeed(actionResponse(status));
			expect(
				await waitForAnswer(stubHubClient({ getResponse }), MESSAGE_ID, {
					timeoutSeconds: 60,
				}),
			).toMatchObject({
				kind: 'closed',
				response: { status },
			});
		},
	);

	it('retries a dropped connection a bounded number of times, then fails', async () => {
		const getResponse = vi.fn<HubClient['getResponse']>(async () =>
			fail({ kind: 'transport', message: 'ECONNRESET' }),
		);
		const clock = fakeClock();
		const outcome = await waitForAnswer(
			stubHubClient({ getResponse }),
			MESSAGE_ID,
			{ timeoutSeconds: 600, ...clock },
		);
		expect(outcome).toMatchObject({
			kind: 'failed',
			failure: { kind: 'transport' },
		});
		expect(getResponse).toHaveBeenCalledTimes(3);
	});

	it('does not retry an HTTP error from the hub', async () => {
		const getResponse = vi.fn<HubClient['getResponse']>(async () =>
			fail({
				kind: 'http',
				status: 404,
				code: 'not_found',
				message: 'no such message',
				pointer: undefined,
			}),
		);
		const outcome = await waitForAnswer(
			stubHubClient({ getResponse }),
			MESSAGE_ID,
			{ timeoutSeconds: 60 },
		);
		expect(outcome).toMatchObject({ kind: 'failed' });
		expect(getResponse).toHaveBeenCalledTimes(1);
	});

	it('stops when cancelled', async () => {
		const controller = new AbortController();
		const getResponse = async () => {
			controller.abort();
			return succeed(actionResponse('pending'));
		};
		const outcome = await waitForAnswer(
			stubHubClient({ getResponse }),
			MESSAGE_ID,
			{
				timeoutSeconds: 60,
				signal: controller.signal,
			},
		);
		expect(outcome).toEqual({ kind: 'cancelled' });
	});
});

describe('askAndWait', () => {
	function clientWith(overrides: Partial<HubClient>) {
		const rescindMessage = vi.fn<HubClient['rescindMessage']>(async () =>
			succeed(
				actionResponse('rescinded', { rescindReason: 'resolved_elsewhere' }),
			),
		);
		const postMessage = vi.fn<HubClient['postMessage']>(async () =>
			succeed({ wasCreated: true, record: ingestResponse() }),
		);
		return {
			client: stubHubClient({ postMessage, rescindMessage, ...overrides }),
			postMessage,
			rescindMessage,
		};
	}

	it('declares awaitsResponse and returns the answer without withdrawing', async () => {
		const { client, postMessage, rescindMessage } = clientWith({
			getResponse: async () =>
				succeed(
					actionResponse('answered', {
						actionKind: 'choice',
						value: 'Use SQLite',
					}),
				),
		});
		const asked = await askAndWait(client, SAMPLE_MESSAGE, {
			timeoutSeconds: 60,
		});
		expect(asked).toMatchObject({
			ok: true,
			value: { outcome: { kind: 'answered' } },
		});
		expect(postMessage.mock.calls[0]?.[0]).toMatchObject({
			awaitsResponse: true,
		});
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('leaves the message open when the deadline passes', async () => {
		const clock = fakeClock();
		const { client, rescindMessage } = clientWith({
			getResponse: async (_id, seconds) => {
				clock.advance(seconds);
				return succeed(actionResponse('pending'));
			},
		});
		const asked = await askAndWait(client, SAMPLE_MESSAGE, {
			timeoutSeconds: 60,
			...clock,
		});
		expect(asked).toMatchObject({
			ok: true,
			value: { outcome: { kind: 'timed_out' } },
		});
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('leaves the message open when the wait is cancelled', async () => {
		const controller = new AbortController();
		const { client, rescindMessage } = clientWith({
			getResponse: async () => {
				controller.abort();
				return succeed(actionResponse('pending'));
			},
		});
		const asked = await askAndWait(client, SAMPLE_MESSAGE, {
			timeoutSeconds: 60,
			signal: controller.signal,
		});
		expect(asked).toMatchObject({
			ok: true,
			value: { outcome: { kind: 'cancelled' } },
		});
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('leaves the message open when the wait fails', async () => {
		const { client, rescindMessage } = clientWith({
			getResponse: async () =>
				fail({
					kind: 'http',
					status: 500,
					code: 'internal',
					message: 'boom',
					pointer: undefined,
				}),
		});
		const asked = await askAndWait(client, SAMPLE_MESSAGE, {
			timeoutSeconds: 60,
		});
		expect(asked).toMatchObject({
			ok: true,
			value: { outcome: { kind: 'failed' } },
		});
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('leaves a deferred message open', async () => {
		const { client, rescindMessage } = clientWith({
			getResponse: async () => succeed(actionResponse('deferred')),
		});
		const asked = await askAndWait(client, SAMPLE_MESSAGE, {
			timeoutSeconds: 60,
		});
		expect(asked).toMatchObject({
			ok: true,
			value: { outcome: { kind: 'deferred' } },
		});
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('does not wait when the hub says nobody can answer, and leaves the message open', async () => {
		const getResponse = vi.fn<HubClient['getResponse']>();
		const { client, rescindMessage } = clientWith({
			postMessage: async () =>
				succeed({
					wasCreated: true,
					record: ingestResponse({ shouldWait: false }),
				}),
			getResponse,
		});
		const asked = await askAndWait(client, SAMPLE_MESSAGE, {
			timeoutSeconds: 60,
		});
		expect(asked).toMatchObject({
			ok: true,
			value: { outcome: { kind: 'not_waiting' } },
		});
		expect(getResponse).not.toHaveBeenCalled();
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('returns muted at once when the hub reports a mute, and never withdraws the message', async () => {
		const getResponse = vi.fn<HubClient['getResponse']>();
		const { client, rescindMessage } = clientWith({
			postMessage: async () =>
				succeed({
					wasCreated: true,
					record: ingestResponse({
						shouldWait: false,
						warnings: [
							{
								code: 'muted',
								message:
									"Delivered to the inbox without a ping: topic 'Release 1.4' is muted",
							},
						],
					}),
				}),
			getResponse,
		});
		const asked = await askAndWait(client, SAMPLE_MESSAGE, {
			timeoutSeconds: 60,
		});
		expect(asked).toMatchObject({
			ok: true,
			value: {
				outcome: {
					kind: 'muted',
					messageId: MESSAGE_ID,
					detail:
						"Delivered to the inbox without a ping: topic 'Release 1.4' is muted",
				},
			},
		});
		expect(getResponse).not.toHaveBeenCalled();
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('returns merged, naming the other message, when the ask was folded into one that is already open', async () => {
		const getResponse = vi.fn<HubClient['getResponse']>();
		const { client, rescindMessage } = clientWith({
			postMessage: async () =>
				succeed({
					wasCreated: false,
					record: ingestResponse({ shouldWait: false }),
				}),
			getResponse,
		});
		const asked = await askAndWait(client, SAMPLE_MESSAGE, {
			timeoutSeconds: 60,
		});
		expect(asked).toMatchObject({
			ok: true,
			value: { outcome: { kind: 'merged', messageId: MESSAGE_ID } },
		});
		expect(getResponse).not.toHaveBeenCalled();
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('still returns muted for a retry that carries the mute warning', async () => {
		const { client } = clientWith({
			postMessage: async () =>
				succeed({
					wasCreated: false,
					record: ingestResponse({
						shouldWait: false,
						warnings: [{ code: 'muted', message: 'Delivered … muted' }],
					}),
				}),
		});
		const asked = await askAndWait(client, SAMPLE_MESSAGE, {
			timeoutSeconds: 60,
		});
		expect(asked).toMatchObject({
			ok: true,
			value: { outcome: { kind: 'muted' } },
		});
	});

	it('reports the real outcome of an idempotent retry that hit a closed message', async () => {
		const { client, rescindMessage } = clientWith({
			postMessage: async () =>
				succeed({
					wasCreated: false,
					record: ingestResponse({ shouldWait: false, state: 'answered' }),
				}),
			getResponse: async () =>
				succeed(actionResponse('answered', { actionKind: 'approve' })),
		});
		const asked = await askAndWait(client, SAMPLE_MESSAGE, {
			timeoutSeconds: 60,
		});
		expect(asked).toMatchObject({
			ok: true,
			value: { outcome: { kind: 'answered' } },
		});
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it('returns the hub failure when the message is refused', async () => {
		const failure = {
			kind: 'http',
			status: 422,
			code: 'schema_invalid',
			message: 'bad',
			pointer: '/title',
		} as const;
		const { client } = clientWith({ postMessage: async () => fail(failure) });
		expect(
			await askAndWait(client, SAMPLE_MESSAGE, { timeoutSeconds: 60 }),
		).toEqual({ ok: false, error: failure });
	});
});
