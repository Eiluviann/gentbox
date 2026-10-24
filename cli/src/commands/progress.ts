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

export const PROGRESS_HELP = `gentbox progress

Tells the hub this conversation moved on: whatever its idle and unanswered blocking items asked
for was handled in the session, so they are withdrawn (resolved_elsewhere). Items someone is
still waiting on stay.`;

export async function runProgress(
	args: string[],
	context: CommandContext,
): Promise<ExitCode> {
	const parsed = parseArguments(args, {} as const);
	if (!parsed.ok) return usageError(context, parsed.error);
	if (parsed.value.positionals.length > 0)
		return usageError(context, 'progress takes no arguments');
	return withHub(context, async (client) => {
		const progressed = await client.reportProgress();
		if (!progressed.ok) return hubError(context, progressed.error);
		printJson(context, { rescinded: progressed.value });
		return EXIT.ok;
	});
}
