import { describe, expect, it, vi } from 'vitest';
import type { HubClient } from '../hub-client.ts';
import { fail, succeed } from '../result.ts';
import { createCommandFixture } from '../test-support/command-fixture.ts';
import { ingestResponse, MESSAGE_ID } from '../test-support/hub-fixtures.ts';
import { isValidMessageJson } from '../test-support/schema-validator.ts';
import { stubHubClient } from '../test-support/stub-hub-client.ts';
import { EXIT } from './command-context.ts';
import { runSend } from './send.ts';

function postingClient(warnings = ingestResponse().warnings) {
	const postMessage = vi.fn<HubClient['postMessage']>(async () =>
		succeed({
			wasCreated: true,
			record: ingestResponse({
				type: 'task_completed',
				blocking: 'non_blocking',
				shouldWait: false,
				warnings,
			}),
		}),
	);
	return { client: stubHubClient({ postMessage }), postMessage };
}

describe('gentbox send', () => {
	it('sends a message built from the flags and prints the hub answer', async () => {
		const { client, postMessage } = postingClient();
		const fixture = createCommandFixture(client);
		const code = await runSend(
			[
				'--type',
				'task_completed',
				'--title',
				'Ingest auth accepts rotated tokens; 12 tests pass',
				'--summary',
				'All green',
				'--scope',
				'goal',
			],
			fixture.context,
		);
		expect(code).toBe(EXIT.ok);
		const sent = postMessage.mock.calls[0]?.[0];
		expect(sent).toMatchObject({
			version: 1,
			source: { agent: 'claude-code' },
			conversation: { id: 'session-1', cwd: '/work/gentbox' },
			type: 'task_completed',
			scope: 'goal',
			title: 'Ingest auth accepts rotated tokens; 12 tests pass',
		});
		expect(isValidMessageJson(sent)).toMatchObject({ isValid: true });
		expect(fixture.stdoutJson()).toMatchObject({
			id: MESSAGE_ID,
			state: 'open',
		});
	});

	it('builds an action plan from repeated --step flags, with a detail after " :: "', async () => {
		const { client, postMessage } = postingClient();
		const code = await runSend(
			[
				'--type',
				'action_required',
				'--title',
				'Sign in to npm so I can publish the CLI package',
				'--step',
				'Open a terminal in any folder',
				'--step',
				'Sign in to npm :: ```bash\nnpm login\n```',
			],
			createCommandFixture(client).context,
		);
		expect(code).toBe(EXIT.ok);
		expect(postMessage.mock.calls[0]?.[0].steps).toEqual([
			{ text: 'Open a terminal in any folder' },
			{ text: 'Sign in to npm', detail: '```bash\nnpm login\n```' },
		]);
	});

	it('requires steps for action_required before contacting the hub', async () => {
		const { client, postMessage } = postingClient();
		const fixture = createCommandFixture(client);
		const code = await runSend(
			['--type', 'action_required', '--title', 'Sign in to npm'],
			fixture.context,
		);
		expect(code).toBe(EXIT.usage);
		expect(fixture.stderr()).toContain('`steps` is required');
		expect(postMessage).not.toHaveBeenCalled();
	});

	it('uses --topic and --thread as inline shortcuts and --rescind-key as rescinds', async () => {
		const { client, postMessage } = postingClient();
		await runSend(
			[
				'--title',
				'Plan replaced',
				'--topic',
				'repo:gentbox',
				'--topic-title',
				'gentbox',
				'--thread',
				'phase-3',
				'--rescind-key',
				'plan-a',
				'--rescind-key',
				'plan-b',
			],
			createCommandFixture(client).context,
		);
		const sent = postMessage.mock.calls[0]?.[0];
		expect(sent?.topic).toEqual({ key: 'repo:gentbox', title: 'gentbox' });
		expect(sent?.thread).toMatchObject({ key: 'phase-3' });
		expect(sent?.rescinds).toEqual({
			reason: 'superseded',
			correlationKeys: ['plan-a', 'plan-b'],
		});
	});

	it('reads the body from a file or from stdin', async () => {
		const { client, postMessage } = postingClient();
		await runSend(
			['--title', 'T', '--body-file', 'body.md'],
			createCommandFixture(client, { files: { 'body.md': '```bash\nls\n```' } })
				.context,
		);
		await runSend(
			['--title', 'T', '--body-file', '-'],
			createCommandFixture(client, { stdin: 'from stdin' }).context,
		);
		expect(postMessage.mock.calls.map((call) => call[0].body)).toEqual([
			'```bash\nls\n```',
			'from stdin',
		]);
	});

	it("passes the hub warnings to stderr in the hub's words", async () => {
		const warning = {
			code: 'title_looks_like_command',
			field: 'title',
			message: 'The title looked like a command and was moved into the body.',
		} as const;
		const { client } = postingClient([warning]);
		const fixture = createCommandFixture(client);
		await runSend(['--title', 'cargo test -p ingest'], fixture.context);
		expect(fixture.stderr()).toBe(
			`warning [title_looks_like_command] title: ${warning.message}\n`,
		);
	});

	it.each([
		[['--type', 'urgent', '--title', 'x'], '--type must be one of'],
		[[], '--title is required'],
		[['--title', 'x', '--bogus'], 'bogus'],
		[
			['--title', 'x', '--type', 'generic', '--blocking', 'blocking'],
			'/blocking',
		],
		[['--title', 'x', '--body', 'a', '--body-file', 'b.md'], 'not both'],
	])('rejects %j as a usage error', async (args, expected) => {
		const { client, postMessage } = postingClient();
		const fixture = createCommandFixture(client, { files: { 'b.md': 'b' } });
		expect(await runSend(args, fixture.context)).toBe(EXIT.usage);
		expect(fixture.stderr()).toContain(expected);
		expect(postMessage).not.toHaveBeenCalled();
	});

	it("exits 1 with the hub's code and pointer when the hub refuses", async () => {
		const client = stubHubClient({
			postMessage: async () =>
				fail({
					kind: 'http',
					status: 422,
					code: 'schema_invalid',
					message: 'too long',
					pointer: '/title',
				}),
		});
		const fixture = createCommandFixture(client);
		expect(await runSend(['--title', 'x'], fixture.context)).toBe(EXIT.failed);
		expect(fixture.stderr()).toContain('schema_invalid at /title: too long');
	});
});
