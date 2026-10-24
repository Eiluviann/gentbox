import { askAndWait } from '../ask-flow.ts';
import { buildAnswerActions, buildMessage } from '../build-message.ts';
import { reportOutcome } from './answer-output.ts';
import {
	type CommandContext,
	type ExitCode,
	hubError,
	messageErrors,
	parseArguments,
	printWarnings,
	usageError,
	withHub,
} from './command-context.ts';
import {
	MESSAGE_OPTIONS,
	MESSAGE_OPTIONS_HELP,
	readMessageDraft,
} from './message-options.ts';

export const DEFAULT_ASK_TIMEOUT_SECONDS = 300;

export const ASK_HELP = `gentbox ask --title <text> [options]

Sends a message that awaits a response and waits for the answer.
Prints the answer as JSON. Exit codes: 0 answered, 3 deferred, 4 no answer
(timed out, dismissed, withdrawn, expired), 1 error. On a timeout the message is withdrawn.

  --choice <label>          A choice button; repeatable (up to 4). Without any, a decision
                            offers Approve and Deny.
  --reply                   Also offer a free-text Reply.
  --timeout <seconds>       How long to wait (default ${DEFAULT_ASK_TIMEOUT_SECONDS}).

Defaults: --type decision_required, --blocking blocking. Use --blocking deferrable when you can
carry on with other work if the user answers Defer.

${MESSAGE_OPTIONS_HELP}`;

const ASK_OPTIONS = {
	...MESSAGE_OPTIONS,
	choice: { type: 'string', multiple: true },
	reply: { type: 'boolean' },
	timeout: { type: 'string' },
} as const;

export async function runAsk(
	args: string[],
	context: CommandContext,
): Promise<ExitCode> {
	const parsed = parseArguments(args, ASK_OPTIONS);
	if (!parsed.ok) return usageError(context, parsed.error);
	if (parsed.value.positionals.length > 0)
		return usageError(
			context,
			'ask takes no positional arguments; use --title',
		);
	const timeoutSeconds = parseTimeout(parsed.value.values.timeout);
	if (timeoutSeconds === undefined)
		return usageError(
			context,
			'--timeout is a whole number of seconds, 1 to 3600',
		);
	const draft = await readMessageDraft(parsed.value.values, context);
	if (!draft.ok) return messageErrors(context, draft.error);
	const type = draft.value.type ?? 'decision_required';
	const message = buildMessage(
		{
			...draft.value,
			type,
			blocking: draft.value.blocking ?? 'blocking',
			actions: buildAnswerActions(type, {
				choices: parsed.value.values.choice ?? [],
				allowReply: parsed.value.values.reply === true,
			}),
		},
		context.sender,
	);
	if (!message.ok) return messageErrors(context, message.error);
	return withHub(context, async (client) => {
		const asked = await askAndWait(client, message.value, { timeoutSeconds });
		if (!asked.ok) return hubError(context, asked.error);
		printWarnings(context, asked.value.ingest.warnings);
		return reportOutcome(context, asked.value.ingest.id, asked.value.outcome, {
			didRescind: asked.value.didRescind,
		});
	});
}

export function parseTimeout(text: string | undefined): number | undefined {
	if (text === undefined) return DEFAULT_ASK_TIMEOUT_SECONDS;
	if (!/^[0-9]{1,4}$/.test(text)) return undefined;
	const seconds = Number(text);
	return seconds >= 1 && seconds <= 3600 ? seconds : undefined;
}
