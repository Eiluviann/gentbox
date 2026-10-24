import { describe, expect, it, vi } from 'vitest';
import type { HubClient } from './hub-client.ts';
import { runCli } from './main.ts';
import { succeed } from './result.ts';
import { environmentOf } from './test-support/environment-of.ts';
import { ingestResponse } from './test-support/hub-fixtures.ts';
import { stubHubClient } from './test-support/stub-hub-client.ts';

function run(
	args: string[],
	client: HubClient,
	environment: NodeJS.ProcessEnv = {},
) {
	let out = '';
	let err = '';
	const connect = vi.fn(async () => succeed(client));
	const code = runCli(args, {
		environment,
		workingDirectory: '/work/gentbox',
		writeOut: (text) => {
			out += text;
		},
		writeError: (text) => {
			err += text;
		},
		readStdin: async () => '',
		connect,
	});
	return { code, out: () => out, err: () => err, connect };
}

describe('runCli', () => {
	it('prints the overview with no command and exits with a usage code', async () => {
		const result = run([], stubHubClient());
		expect(await result.code).toBe(2);
		expect(result.err()).toContain('Commands:');
		expect(result.connect).not.toHaveBeenCalled();
	});

	it.each([
		'send',
		'ask',
		'wait',
		'rescind',
		'progress',
		'focus',
		'topic',
		'thread',
	])('prints help for %s without contacting the hub', async (command) => {
		const viaHelp = run(['help', command], stubHubClient());
		const viaFlag = run([command, '--help'], stubHubClient());
		expect(await viaHelp.code).toBe(0);
		expect(await viaFlag.code).toBe(0);
		expect(viaHelp.out()).toContain(`gentbox ${command}`);
		expect(viaHelp.connect).not.toHaveBeenCalled();
	});

	it('has no answer or list command: the sender token cannot do either', async () => {
		for (const command of ['answer', 'list']) {
			const result = run([command], stubHubClient());
			expect(await result.code).toBe(2);
			expect(result.err()).toContain('unknown command');
		}
	});

	it('takes the identity from the global flags before the command, over the environment', async () => {
		const postMessage = vi.fn<HubClient['postMessage']>(async () =>
			succeed({ wasCreated: true, record: ingestResponse() }),
		);
		const result = run(
			[
				'--agent',
				'codex',
				'--conversation',
				'abc-1',
				'send',
				'--title',
				'Hello',
			],
			stubHubClient({ postMessage }),
			environmentOf(
				['GENTBOX_AGENT', 'claude-code'],
				['GENTBOX_CONVERSATION', 'from-env'],
			),
		);
		expect(await result.code).toBe(0);
		expect(postMessage.mock.calls[0]?.[0]).toMatchObject({
			source: { agent: 'codex' },
			conversation: { id: 'abc-1' },
		});
	});

	it('defaults to the cli agent and one conversation per folder', async () => {
		const postMessage = vi.fn<HubClient['postMessage']>(async () =>
			succeed({ wasCreated: true, record: ingestResponse() }),
		);
		await run(['send', '--title', 'One'], stubHubClient({ postMessage })).code;
		await run(['send', '--title', 'Two'], stubHubClient({ postMessage })).code;
		const [first, second] = postMessage.mock.calls.map((call) => call[0]);
		expect(first?.source.agent).toBe('cli');
		expect(first?.conversation.id).toMatch(/^cli-[0-9a-f]{12}$/);
		expect(second?.conversation.id).toBe(first?.conversation.id);
	});

	it('refuses a malformed agent id instead of posting under it', async () => {
		const result = run(
			['--agent', 'Not Valid', 'send', '--title', 'x'],
			stubHubClient(),
		);
		expect(await result.code).toBe(2);
		expect(result.err()).toContain('GENTBOX_AGENT');
		expect(result.connect).not.toHaveBeenCalled();
	});

	it('rejects a global flag without a value', async () => {
		expect(await run(['--agent'], stubHubClient()).code).toBe(2);
	});
});
