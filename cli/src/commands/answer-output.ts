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
	extra: { didRescind?: boolean } = {},
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
				...withdrawn(extra),
			});
			return EXIT.noAnswer;
		case 'not_waiting':
			printJson(context, {
				messageId,
				status: 'no_answer',
				reason: 'nobody_can_answer_now',
				...withdrawn(extra),
			});
			return EXIT.noAnswer;
		case 'cancelled':
			printJson(context, {
				messageId,
				status: 'no_answer',
				reason: 'cancelled',
				...withdrawn(extra),
			});
			return EXIT.noAnswer;
		case 'failed':
			printJson(context, { messageId, status: 'unknown', ...withdrawn(extra) });
			return hubError(context, outcome.failure);
	}
}

function withdrawn(extra: { didRescind?: boolean }): {
	messageWithdrawn: boolean;
} {
	return { messageWithdrawn: extra.didRescind === true };
}
