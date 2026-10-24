import { isUlid, rescindReasonSchema } from '../message-types.ts';
import {
	type CommandContext,
	EXIT,
	type ExitCode,
	hubError,
	parseArguments,
	printJson,
	usageError,
	withHub,
} from './command-context.ts';

export const RESCIND_HELP = `gentbox rescind <message-id> [--reason <reason>]
gentbox rescind --key <correlation-key>... [--reason <reason>]

Withdraws your own open message(s): a message by id, or every open message of this conversation
with the given correlation keys. Reasons: resolved_elsewhere (default for an id), obsolete,
superseded (default for keys).

Withdrawn means you asked: gentbox never withdraws a message for you, not on a timeout, a cancel or a
muted topic.`;

export async function runRescind(
	args: string[],
	context: CommandContext,
): Promise<ExitCode> {
	const parsed = parseArguments(args, {
		reason: { type: 'string' },
		key: { type: 'string', multiple: true },
	} as const);
	if (!parsed.ok) return usageError(context, parsed.error);
	const { values, positionals } = parsed.value;
	const reason =
		values.reason === undefined
			? undefined
			: rescindReasonSchema.safeParse(values.reason);
	if (reason !== undefined && !reason.success) {
		return usageError(
			context,
			`--reason must be one of: ${rescindReasonSchema.options.join(', ')}`,
		);
	}
	const keys = values.key ?? [];
	const [messageId, ...rest] = positionals;
	if ((messageId === undefined) === (keys.length === 0) || rest.length > 0) {
		return usageError(
			context,
			'give either one message id or one or more --key, not both',
		);
	}
	if (messageId !== undefined && !isUlid(messageId))
		return usageError(context, `"${messageId}" is not a message id`);
	return withHub(context, async (client) => {
		if (messageId !== undefined) {
			const rescinded = await client.rescindMessage(
				messageId,
				reason?.data ?? 'resolved_elsewhere',
			);
			if (!rescinded.ok) return hubError(context, rescinded.error);
			printJson(context, rescinded.value);
			return EXIT.ok;
		}
		const rescinded = await client.rescindByCorrelationKeys({
			reason: reason?.data ?? 'superseded',
			correlationKeys: keys,
		});
		if (!rescinded.ok) return hubError(context, rescinded.error);
		printJson(context, { rescinded: rescinded.value });
		return EXIT.ok;
	});
}
