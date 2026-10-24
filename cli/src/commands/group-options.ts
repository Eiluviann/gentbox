import type { HubClient } from '../hub-client.ts';
import {
	type CloseRequest,
	isUlid,
	rescindReasonSchema,
} from '../message-types.ts';
import { fail, type Result, succeed } from '../result.ts';

export const CLOSE_OPTIONS = {
	'rescind-open': { type: 'boolean' },
	reason: { type: 'string' },
} as const;

export const CLOSE_OPTIONS_HELP = `  --rescind-open            Withdraw your own open messages first. The close is refused (and prints the count)
                            if other senders' messages stay open; the user has to handle those.
  --reason <reason>         With --rescind-open: resolved_elsewhere | obsolete (default) | superseded.`;

/** `--rescind-open` and `--reason` as the hub's close request. */
export function readCloseRequest(values: {
	'rescind-open'?: boolean | undefined;
	reason?: string | undefined;
}): Result<CloseRequest, string> {
	const reason =
		values.reason === undefined
			? undefined
			: rescindReasonSchema.safeParse(values.reason);
	if (reason !== undefined && !reason.success) {
		return fail(
			`--reason must be one of: ${rescindReasonSchema.options.join(', ')}`,
		);
	}
	if (values.reason !== undefined && values['rescind-open'] !== true)
		return fail('--reason needs --rescind-open');
	return succeed({
		rescindOpenMessages: values['rescind-open'],
		reason: reason?.data,
	});
}

/** A topic id as given, or the id of the open or closed topic with this key. */
export async function resolveTopicId(
	client: HubClient,
	reference: string,
): Promise<Result<string, string>> {
	if (isUlid(reference)) return succeed(reference);
	const listed = await client.listTopics(undefined);
	if (!listed.ok)
		return fail(
			`cannot look up topic "${reference}": ${listed.error.kind === 'http' ? listed.error.message : listed.error.message}`,
		);
	const topic = listed.value.find((candidate) => candidate.key === reference);
	return topic === undefined
		? fail(`no topic has the key "${reference}"`)
		: succeed(topic.id);
}

/** A thread id as given, or the id of the thread with this key inside `topicReference`. */
export async function resolveThreadId(
	client: HubClient,
	reference: string,
	topicReference: string | undefined,
): Promise<Result<string, string>> {
	if (isUlid(reference)) return succeed(reference);
	if (topicReference === undefined)
		return fail(`"${reference}" is a thread key; name its topic too`);
	const topicId = await resolveTopicId(client, topicReference);
	if (!topicId.ok) return topicId;
	const listed = await client.listThreads(topicId.value, undefined);
	if (!listed.ok)
		return fail(
			`cannot look up thread "${reference}": ${listed.error.message}`,
		);
	const thread = listed.value.find((candidate) => candidate.key === reference);
	return thread === undefined
		? fail(`no thread has the key "${reference}" in that topic`)
		: succeed(thread.id);
}
