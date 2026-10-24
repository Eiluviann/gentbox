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

export const CLOSE_OPTIONS_HELP = `  --rescind-open            Also withdraw every open message in it.
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
