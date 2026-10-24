import { readFileSync } from 'node:fs';
import type { SenderContext } from './build-message.ts';
import { ASK_HELP, runAsk } from './commands/ask.ts';
import type { CommandContext, ExitCode } from './commands/command-context.ts';
import { EXIT } from './commands/command-context.ts';
import { FOCUS_HELP, runFocus } from './commands/focus.ts';
import { PROGRESS_HELP, runProgress } from './commands/progress.ts';
import { RESCIND_HELP, runRescind } from './commands/rescind.ts';
import { runSend, SEND_HELP } from './commands/send.ts';
import { runThread, THREAD_HELP } from './commands/thread.ts';
import { runTopic, TOPIC_HELP } from './commands/topic.ts';
import { runWait, WAIT_HELP } from './commands/wait.ts';
import { readDiscoveryEnvironment } from './discovery.ts';
import { connectToHub } from './hub-client.ts';
import {
	conversationIdForFolder,
	readSenderContext,
} from './sender-context.ts';

export type CliIo = {
	environment: NodeJS.ProcessEnv;
	workingDirectory: string;
	writeOut: (text: string) => void;
	writeError: (text: string) => void;
	readStdin: () => Promise<string>;
	/** Overrides for tests; the real CLI connects to the running hub. */
	connect?: CommandContext['connect'];
};

const HELP_BY_COMMAND: Record<string, string> = {
	send: SEND_HELP,
	ask: ASK_HELP,
	wait: WAIT_HELP,
	rescind: RESCIND_HELP,
	progress: PROGRESS_HELP,
	focus: FOCUS_HELP,
	topic: TOPIC_HELP,
	thread: THREAD_HELP,
};

const OVERVIEW = `gentbox: send messages to your Gentbox inbox and wait for answers

Usage: gentbox [--agent <id>] [--conversation <id>] <command> [options]

Commands:
  send      Send a message and return
  ask       Send a message and wait for the answer
  wait      Wait for the answer to a message you sent
  rescind   Withdraw your open message(s)
  progress  Report that the conversation moved on
  focus     Tell Gentbox a macOS Focus is on or off
  topic     Create, list, close and reopen topics
  thread    Create, list, close, reopen and move threads

Run \`gentbox help <command>\` for a command's options.

Identity (self-asserted): --agent or GENTBOX_AGENT (default "cli"); --conversation or
GENTBOX_CONVERSATION (default: one per working folder). GENTBOX_API_DIR points at the hub's API folder.
The CLI cannot answer or list messages: the sender token does not allow it.`;

/** Runs one CLI invocation and returns its exit code. */
export async function runCli(args: string[], io: CliIo): Promise<ExitCode> {
	const { globals, rest } = splitGlobalOptions(args);
	if (!globals.ok) return fail(io, globals.error);
	const [command, ...commandArgs] = rest;
	if (command === undefined || command === '--help' || command === '-h') {
		if (command === undefined) {
			io.writeError(`${OVERVIEW}\n`);
			return EXIT.usage;
		}
		io.writeOut(`${OVERVIEW}\n`);
		return EXIT.ok;
	}
	if (command === 'help') return printHelp(commandArgs[0], io);
	if (commandArgs.includes('--help') || commandArgs.includes('-h'))
		return printHelp(command, io);
	const runner = RUNNERS[command];
	if (runner === undefined)
		return fail(io, `unknown command "${command}"; run \`gentbox help\``);
	const environment = { ...io.environment };
	if (globals.value.agent !== undefined)
		environment.GENTBOX_AGENT = globals.value.agent;
	if (globals.value.conversation !== undefined)
		environment.GENTBOX_CONVERSATION = globals.value.conversation;
	let sender: SenderContext;
	try {
		sender = readSenderContext(
			environment,
			{
				agent: 'cli',
				conversationId: () => conversationIdForFolder(io.workingDirectory),
			},
			io.workingDirectory,
		);
	} catch (error) {
		return fail(io, error instanceof Error ? error.message : String(error));
	}
	return runner(commandArgs, createCommandContext(io, environment, sender));
}

const RUNNERS: Record<
	string,
	(args: string[], context: CommandContext) => Promise<ExitCode>
> = {
	send: runSend,
	ask: runAsk,
	wait: runWait,
	rescind: runRescind,
	progress: runProgress,
	focus: runFocus,
	topic: runTopic,
	thread: runThread,
};

function createCommandContext(
	io: CliIo,
	environment: NodeJS.ProcessEnv,
	sender: SenderContext,
): CommandContext {
	return {
		connect:
			io.connect ??
			(() =>
				connectToHub({
					environment: readDiscoveryEnvironment(environment),
					identity: {
						agent: sender.agent,
						conversationId: sender.conversationId,
					},
				})),
		sender,
		readFile: (path) => readFileSync(path, 'utf8'),
		readStdin: io.readStdin,
		writeOut: io.writeOut,
		writeError: io.writeError,
	};
}

type GlobalOptions = {
	agent: string | undefined;
	conversation: string | undefined;
};

/** `--agent` and `--conversation` come before the command name. */
function splitGlobalOptions(args: string[]): {
	globals: { ok: true; value: GlobalOptions } | { ok: false; error: string };
	rest: string[];
} {
	const value: GlobalOptions = { agent: undefined, conversation: undefined };
	let index = 0;
	while (index < args.length) {
		const flag = args[index];
		if (flag !== '--agent' && flag !== '--conversation') break;
		const argument = args[index + 1];
		if (argument === undefined || argument.startsWith('--')) {
			return {
				globals: { ok: false, error: `${flag} needs a value` },
				rest: [],
			};
		}
		if (flag === '--agent') value.agent = argument;
		else value.conversation = argument;
		index += 2;
	}
	return { globals: { ok: true, value }, rest: args.slice(index) };
}

function printHelp(command: string | undefined, io: CliIo): ExitCode {
	if (command === undefined) {
		io.writeOut(`${OVERVIEW}\n`);
		return EXIT.ok;
	}
	const help = HELP_BY_COMMAND[command];
	if (help === undefined) return fail(io, `unknown command "${command}"`);
	io.writeOut(`${help}\n`);
	return EXIT.ok;
}

function fail(io: CliIo, message: string): ExitCode {
	io.writeError(`gentbox: ${message}\n`);
	return EXIT.usage;
}
