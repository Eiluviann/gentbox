import type { SenderContext } from '../build-message.ts';
import type { CommandContext } from '../commands/command-context.ts';
import type { HubClient } from '../hub-client.ts';
import { succeed } from '../result.ts';

export const TEST_SENDER: SenderContext = {
	agent: 'claude-code',
	displayName: undefined,
	conversationId: 'session-1',
	conversationTitle: undefined,
	workingDirectory: '/work/gentbox',
	now: () => new Date('2026-10-24T12:00:00Z'),
};

export type CommandFixture = {
	context: CommandContext;
	stdout: () => string;
	stderr: () => string;
	stdoutJson: () => unknown;
};

/** A command context over a stub client, with captured output and in-memory files. */
export function createCommandFixture(
	client: HubClient,
	options: { files?: Record<string, string>; stdin?: string } = {},
): CommandFixture {
	let out = '';
	let err = '';
	const context: CommandContext = {
		connect: async () => succeed(client),
		sender: TEST_SENDER,
		readFile: (path) => {
			const contents = options.files?.[path];
			if (contents === undefined) throw new Error(`no such file: ${path}`);
			return contents;
		},
		readStdin: async () => options.stdin ?? '',
		writeOut: (text) => {
			out += text;
		},
		writeError: (text) => {
			err += text;
		},
	};
	return {
		context,
		stdout: () => out,
		stderr: () => err,
		stdoutJson: () => JSON.parse(out),
	};
}
