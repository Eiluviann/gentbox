import {
	type CommandContext,
	EXIT,
	type ExitCode,
	hubError,
	parseArguments,
	usageError,
	withHub,
} from './command-context.ts';

export const FOCUS_HELP = `gentbox focus on|off

Tells Gentbox a macOS Focus is on or off (for a Shortcuts automation, when the app cannot read
Focus itself).`;

export async function runFocus(
	args: string[],
	context: CommandContext,
): Promise<ExitCode> {
	const parsed = parseArguments(args, {} as const);
	if (!parsed.ok) return usageError(context, parsed.error);
	const [state, ...rest] = parsed.value.positionals;
	if ((state !== 'on' && state !== 'off') || rest.length > 0)
		return usageError(context, 'use `gentbox focus on` or `gentbox focus off`');
	return withHub(context, async (client) => {
		const set = await client.setFocus(state === 'on');
		return set.ok ? EXIT.ok : hubError(context, set.error);
	});
}
