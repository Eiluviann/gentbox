import { type ParseArgsOptionsConfig, parseArgs } from 'node:util';
import type { SenderContext } from '../build-message.ts';
import {
	describeCloseFailure,
	describeHubFailure,
	type HubClient,
	type HubFailure,
} from '../hub-client.ts';
import type { IngestWarning } from '../message-types.ts';
import { fail, type Result, succeed } from '../result.ts';

/** Exit codes: scripts and agents branch on them. */
export const EXIT = {
	/** Sent, or answered. */
	ok: 0,
	/** The hub refused, was unreachable or answered something unexpected. */
	failed: 1,
	/** The command line is wrong. */
	usage: 2,
	/** The user chose Defer: keep working on what needs no input. */
	deferred: 3,
	/** Timed out, dismissed, withdrawn, expired or gone: there is no answer. */
	noAnswer: 4,
} as const;
export type ExitCode = (typeof EXIT)[keyof typeof EXIT];

/** What a command needs from its surroundings; tests pass fakes. */
export type CommandContext = {
	/** Connects on first use, so a usage error or `--help` never needs a running hub. */
	connect: () => Promise<Result<HubClient, string>>;
	sender: SenderContext;
	readFile: (path: string) => string;
	readStdin: () => Promise<string>;
	writeOut: (text: string) => void;
	writeError: (text: string) => void;
};

export type ParsedArguments<O extends ParseArgsOptionsConfig> = ReturnType<
	typeof parseArgs<{ options: O; allowPositionals: true; allowNegative: true }>
>;

/** `parseArgs` with a bad flag as a `Result` message instead of a throw. */
export function parseArguments<O extends ParseArgsOptionsConfig>(
	args: string[],
	options: O,
): Result<ParsedArguments<O>, string> {
	try {
		return succeed(
			parseArgs({
				args,
				options,
				allowPositionals: true,
				allowNegative: true,
				strict: true,
			}),
		);
	} catch (error) {
		return fail(error instanceof Error ? error.message : String(error));
	}
}

export function printJson(context: CommandContext, value: unknown): void {
	context.writeOut(`${JSON.stringify(value, null, 2)}\n`);
}

/** The hub's `warnings` go to stderr, one line each, in the hub's own words. */
export function printWarnings(
	context: CommandContext,
	warnings: IngestWarning[] | undefined,
): void {
	for (const warning of warnings ?? []) {
		context.writeError(
			`warning [${warning.code}]${warning.field === undefined ? '' : ` ${warning.field}`}: ${warning.message}\n`,
		);
	}
}

export function usageError(context: CommandContext, message: string): ExitCode {
	context.writeError(`gentbox: ${message}\n`);
	return EXIT.usage;
}

export function hubError(
	context: CommandContext,
	failure: HubFailure,
): ExitCode {
	context.writeError(`gentbox: ${describeHubFailure(failure)}\n`);
	return EXIT.failed;
}

/** A refused `topic close` or `thread close`, with the count and what to do about it. */
export function closeError(
	context: CommandContext,
	failure: HubFailure,
	entity: 'topic' | 'thread',
): ExitCode {
	context.writeError(
		`gentbox: ${describeCloseFailure(failure, entity, '--rescind-open')}\n`,
	);
	return EXIT.failed;
}

export function messageErrors(
	context: CommandContext,
	errors: string[],
): ExitCode {
	for (const error of errors) context.writeError(`gentbox: ${error}\n`);
	return EXIT.usage;
}

/** Connects, or prints why not and gives the exit code to return. */
export async function withHub(
	context: CommandContext,
	run: (client: HubClient) => Promise<ExitCode>,
): Promise<ExitCode> {
	const connected = await context.connect();
	if (!connected.ok) {
		context.writeError(`gentbox: ${connected.error}\n`);
		return EXIT.failed;
	}
	return run(connected.value);
}
