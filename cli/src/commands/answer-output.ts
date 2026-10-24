import type { AskOutcome } from '../ask-flow.ts';
import {
	type CommandContext,
	EXIT,
	type ExitCode,
	hubError,
	printJson,
} from './command-context.ts';

/** Prints how a wait ended as JSON and returns the exit code for it. */
export function reportOutcome(
	context: CommandContext,
	messageId: string,
	outcome: AskOutcome,
): ExitCode {
	switch (outcome.kind) {
		case 'answered': {
			const { actionId, actionKind, value, answeredVia, answeredAt } =
				outcome.response;
			printJson(context, {
				messageId,
				status: 'answered',
				actionId,
				actionKind,
				value,
				answeredVia,
				answeredAt,
			});
			return EXIT.ok;
		}
		case 'deferred':
			printJson(context, {
				messageId,
				status: 'deferred',
				note: 'The user chose Defer: keep working on what needs no input. The message stays open; run `gentbox wait` for the answer later.',
			});
			return EXIT.deferred;
		case 'closed':
			printJson(context, {
				messageId,
				status: outcome.response.status,
				rescindReason: outcome.response.rescindReason,
				note: 'No answer will come; fall back to your own prompt.',
			});
			return EXIT.noAnswer;
		case 'timed_out':
			printJson(context, {
				messageId,
				status: 'no_answer',
				reason: 'timed_out',
				note: 'No answer yet; the message stays open. Run `gentbox wait` to collect the answer later, or ask in the conversation and `gentbox rescind` the message once that is handled.',
			});
			return EXIT.noAnswer;
		case 'not_waiting':
			printJson(context, {
				messageId,
				status: 'no_answer',
				reason: 'nobody_can_answer_now',
				note: 'No channel can take an answer right now; the message stays open in the inbox. Ask in the conversation and `gentbox rescind` the message once that is handled.',
			});
			return EXIT.noAnswer;
		case 'muted':
			printJson(context, {
				messageId,
				status: 'no_answer',
				reason: 'muted',
				note: `The message was delivered to the inbox, but its topic or thread is muted, so the user wasn't pinged and may answer later. The message stays open. Run \`gentbox wait\` to collect the answer, or ask in the conversation and \`gentbox rescind\` the message (--reason resolved_elsewhere) once that is handled.`,
			});
			return EXIT.noAnswer;
		case 'cancelled':
			printJson(context, {
				messageId,
				status: 'no_answer',
				reason: 'cancelled',
				note: 'The wait was cancelled; the message stays open.',
			});
			return EXIT.noAnswer;
		case 'failed':
			printJson(context, {
				messageId,
				status: 'unknown',
				note: 'The answer is unknown; the message stays open.',
			});
			return hubError(context, outcome.failure);
	}
}
