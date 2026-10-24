import type { HubClient, HubFailure } from './hub-client.ts';
import type {
	ActionResponse,
	IngestResponse,
	MessageV1,
} from './message-types.ts';
import { fail, type Result, succeed } from './result.ts';

/** How a wait for an answer ended. `pending` never ends a wait: it means poll again. */
export type WaitOutcome =
	| { kind: 'answered'; response: ActionResponse }
	/** The user chose Defer: keep working on what needs no input. The message stays open. */
	| { kind: 'deferred'; response: ActionResponse }
	/** `dismissed`, `rescinded`, `expired` or `gone`: no answer will come. */
	| { kind: 'closed'; response: ActionResponse }
	| { kind: 'timed_out' }
	| { kind: 'cancelled' }
	| { kind: 'failed'; failure: HubFailure };

export type WaitOptions = {
	timeoutSeconds: number;
	signal?: AbortSignal | undefined;
	/** Injected for tests. */
	now?: () => number;
	sleep?: (milliseconds: number) => Promise<void>;
};

/** The hub holds a poll for at most 55 s; stay a little under so a slow link never trips the client timeout. */
const POLL_SECONDS = 50;
const MAX_CONSECUTIVE_FAILURES = 3;
const RETRY_DELAY_MILLISECONDS = 1000;

/**
 * Long-polls `GET /v1/messages/{id}/response` until an answer, `deferred`, a
 * closed status or the deadline. A dropped connection is retried a few times
 * (the hub keeps the message); an HTTP error from the hub ends the wait.
 */
export async function waitForAnswer(
	client: HubClient,
	messageId: string,
	options: WaitOptions,
): Promise<WaitOutcome> {
	const now = options.now ?? Date.now;
	const sleep = options.sleep ?? defaultSleep;
	const deadline = now() + options.timeoutSeconds * 1000;
	const failures = { consecutive: 0 };
	while (true) {
		const remainingSeconds = Math.ceil((deadline - now()) / 1000);
		const step = await pollOnce(
			client,
			messageId,
			remainingSeconds,
			options.signal,
			failures,
		);
		if (step.kind === 'finished') return step.outcome;
		if (step.shouldPause) await sleep(RETRY_DELAY_MILLISECONDS);
	}
}

type PollStep =
	| { kind: 'finished'; outcome: WaitOutcome }
	| { kind: 'again'; shouldPause: boolean };

/** One long-poll. `failures` counts consecutive dropped connections across calls. */
async function pollOnce(
	client: HubClient,
	messageId: string,
	remainingSeconds: number,
	signal: AbortSignal | undefined,
	failures: { consecutive: number },
): Promise<PollStep> {
	if (signal?.aborted)
		return { kind: 'finished', outcome: { kind: 'cancelled' } };
	if (remainingSeconds <= 0)
		return { kind: 'finished', outcome: { kind: 'timed_out' } };
	const polled = await client.getResponse(
		messageId,
		Math.min(POLL_SECONDS, remainingSeconds),
		signal,
	);
	if (signal?.aborted)
		return { kind: 'finished', outcome: { kind: 'cancelled' } };
	if (!polled.ok) {
		failures.consecutive++;
		if (!canRetry(polled.error, failures.consecutive)) {
			return {
				kind: 'finished',
				outcome: { kind: 'failed', failure: polled.error },
			};
		}
		return { kind: 'again', shouldPause: true };
	}
	failures.consecutive = 0;
	const outcome = outcomeOf(polled.value);
	return outcome === undefined
		? { kind: 'again', shouldPause: false }
		: { kind: 'finished', outcome };
}

function canRetry(failure: HubFailure, consecutiveFailures: number): boolean {
	return (
		failure.kind === 'transport' &&
		consecutiveFailures < MAX_CONSECUTIVE_FAILURES
	);
}

export type AskOutcome = WaitOutcome | { kind: 'not_waiting' };

export type AskResult = {
	ingest: IngestResponse;
	outcome: AskOutcome;
	/** True when this call withdrew the message because the sender stopped waiting. */
	didRescind: boolean;
};

/**
 * Sends a message that awaits a response and waits for it. When the sender
 * stops waiting (deadline, cancel, error, or the hub says nobody can answer)
 * the message is withdrawn as `resolved_elsewhere`, as the architecture
 * requires, so no stale popup is left behind; a deferral leaves it open.
 */
export async function askAndWait(
	client: HubClient,
	message: MessageV1,
	options: WaitOptions,
): Promise<Result<AskResult, HubFailure>> {
	const posted = await client.postMessage({ ...message, awaitsResponse: true });
	if (!posted.ok) return fail(posted.error);
	const ingest = posted.value.record;
	if (!ingest.shouldWait) {
		return succeed(await settleWithoutWaiting(client, ingest));
	}
	const outcome = await waitForAnswer(client, ingest.id, options);
	const shouldWithdraw =
		outcome.kind === 'timed_out' ||
		outcome.kind === 'cancelled' ||
		outcome.kind === 'failed';
	const didRescind = shouldWithdraw && (await withdraw(client, ingest.id));
	return succeed({ ingest, outcome, didRescind });
}

/** `shouldWait: false`: either nobody can answer now, or an idempotent retry hit a message that already closed. */
async function settleWithoutWaiting(
	client: HubClient,
	ingest: IngestResponse,
): Promise<AskResult> {
	const hasClosed = ['answered', 'dismissed', 'rescinded', 'expired'].includes(
		ingest.state,
	);
	if (hasClosed) {
		const polled = await client.getResponse(ingest.id, 0);
		const outcome = polled.ok ? outcomeOf(polled.value) : undefined;
		if (outcome !== undefined) return { ingest, outcome, didRescind: false };
	}
	const didRescind = await withdraw(client, ingest.id);
	return { ingest, outcome: { kind: 'not_waiting' }, didRescind };
}

function outcomeOf(response: ActionResponse): WaitOutcome | undefined {
	switch (response.status) {
		case 'pending':
			return undefined;
		case 'answered':
			return { kind: 'answered', response };
		case 'deferred':
			return { kind: 'deferred', response };
		case 'dismissed':
		case 'rescinded':
		case 'expired':
		case 'gone':
			return { kind: 'closed', response };
	}
}

async function withdraw(
	client: HubClient,
	messageId: string,
): Promise<boolean> {
	const rescinded = await client.rescindMessage(
		messageId,
		'resolved_elsewhere',
	);
	return rescinded.ok;
}

function defaultSleep(milliseconds: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
