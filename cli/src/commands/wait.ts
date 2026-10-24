import { waitForAnswer } from '../ask-flow.ts';
import { isUlid } from '../message-types.ts';
import { reportOutcome } from './answer-output.ts';
import { DEFAULT_ASK_TIMEOUT_SECONDS, parseTimeout } from './ask.ts';
import {
	type CommandContext,
	type ExitCode,
	parseArguments,
	usageError,
	withHub,
} from './command-context.ts';

export const WAIT_HELP = `gentbox wait <message-id> [--timeout <seconds>]

Waits for the answer to a message you sent (for instance after a deferral). Does not withdraw the
message when the wait ends. Exit codes as for ask: 0 answered, 3 deferred, 4 no answer, 1 error.
Default timeout ${DEFAULT_ASK_TIMEOUT_SECONDS} seconds.`;

export async function runWait(
	args: string[],
	context: CommandContext,
): Promise<ExitCode> {
	const parsed = parseArguments(args, { timeout: { type: 'string' } } as const);
	if (!parsed.ok) return usageError(context, parsed.error);
	const [messageId, ...rest] = parsed.value.positionals;
	if (messageId === undefined || rest.length > 0)
		return usageError(context, 'wait takes one message id');
	if (!isUlid(messageId))
		return usageError(context, `"${messageId}" is not a message id`);
	const timeoutSeconds = parseTimeout(parsed.value.values.timeout);
	if (timeoutSeconds === undefined)
		return usageError(
			context,
			'--timeout is a whole number of seconds, 1 to 3600',
		);
	return withHub(context, async (client) => {
		const outcome = await waitForAnswer(client, messageId, { timeoutSeconds });
		return reportOutcome(context, messageId, outcome);
	});
}
