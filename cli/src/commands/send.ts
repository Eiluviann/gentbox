import { buildMessage } from '../build-message.ts';
import {
	type CommandContext,
	EXIT,
	type ExitCode,
	hubError,
	messageErrors,
	parseArguments,
	printJson,
	printWarnings,
	usageError,
	withHub,
} from './command-context.ts';
import {
	MESSAGE_OPTIONS,
	MESSAGE_OPTIONS_HELP,
	readMessageDraft,
} from './message-options.ts';

export const SEND_HELP = `gentbox send --title <text> [options]

Sends a message and returns at once. Prints the hub's answer as JSON and its warnings to stderr.

${MESSAGE_OPTIONS_HELP}`;

export async function runSend(
	args: string[],
	context: CommandContext,
): Promise<ExitCode> {
	const parsed = parseArguments(args, MESSAGE_OPTIONS);
	if (!parsed.ok) return usageError(context, parsed.error);
	if (parsed.value.positionals.length > 0)
		return usageError(
			context,
			'send takes no positional arguments; use --title',
		);
	const draft = await readMessageDraft(parsed.value.values, context);
	if (!draft.ok) return messageErrors(context, draft.error);
	const message = buildMessage(draft.value, context.sender);
	if (!message.ok) return messageErrors(context, message.error);
	return withHub(context, async (client) => {
		const posted = await client.postMessage(message.value);
		if (!posted.ok) return hubError(context, posted.error);
		printJson(context, posted.value.record);
		printWarnings(context, posted.value.record.warnings);
		return EXIT.ok;
	});
}
