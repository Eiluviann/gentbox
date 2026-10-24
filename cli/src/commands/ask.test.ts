import { describe, expect, it, vi } from 'vitest';
import type { HubClient } from '../hub-client.ts';
import type { ResponseStatus } from '../message-types.ts';
import { succeed } from '../result.ts';
import { createCommandFixture } from '../test-support/command-fixture.ts';
import {
	actionResponse,
	ingestResponse,
	MESSAGE_ID,
} from '../test-support/hub-fixtures.ts';
import { isValidMessageJson } from '../test-support/schema-validator.ts';
import { stubHubClient } from '../test-support/stub-hub-client.ts';
import { runAsk } from './ask.ts';
import { EXIT } from './command-context.ts';

function askingClient(
	response: ReturnType<typeof actionResponse>,
	ingest = ingestResponse(),
) {
	const postMessage = vi.fn<HubClient['postMessage']>(async () =>
		succeed({ wasCreated: true, record: ingest }),
	);
	const rescindMessage = vi.fn<HubClient['rescindMessage']>(async () =>
		succeed(actionResponse('rescinded')),
	);
	const getResponse = vi.fn<HubClient['getResponse']>(async () =>
		succeed(response),
	);
	return {
		client: stubHubClient({ postMessage, rescindMessage, getResponse }),
		postMessage,
		rescindMessage,
		getResponse,
	};
}

describe('gentbox ask', () => {
	it('sends a blocking decision that awaits a response, with Approve and Deny by default', async () => {
		const { client, postMessage } = askingClient(
			actionResponse('answered', {
				actionId: 'approve',
				actionKind: 'approve',
				answeredVia: 'mac',
			}),
		);
		const fixture = createCommandFixture(client);
		const code = await runAsk(
			[
				'--title',
				'Allow Bash to run the ingest tests?',
				'--body',
				'```bash\ncargo test\n```',
			],
			fixture.context,
		);
		expect(code).toBe(EXIT.ok);
		const sent = postMessage.mock.calls[0]?.[0];
		expect(sent).toMatchObject({
			type: 'decision_required',
			blocking: 'blocking',
			awaitsResponse: true,
		});
		expect(sent?.actions?.map((action) => action.kind)).toEqual([
			'approve',
			'deny',
			'open',
		]);
		expect(isValidMessageJson(sent)).toMatchObject({ isValid: true });
		expect(fixture.stdoutJson()).toMatchObject({
			messageId: MESSAGE_ID,
			status: 'answered',
			actionKind: 'approve',
			answeredVia: 'mac',
		});
	});

	it('turns --choice and --reply into buttons and prints the chosen value', async () => {
		const { client, postMessage } = askingClient(
			actionResponse('answered', { actionKind: 'choice', value: 'Use SQLite' }),
		);
		const fixture = createCommandFixture(client);
		const code = await runAsk(
			[
				'--title',
				'Which store should the hub use?',
				'--choice',
				'Use SQLite',
				'--choice',
				'Use Postgres',
				'--reply',
				'--blocking',
				'deferrable',
			],
			fixture.context,
		);
		expect(code).toBe(EXIT.ok);
		expect(postMessage.mock.calls[0]?.[0]).toMatchObject({
			blocking: 'deferrable',
		});
		expect(
			postMessage.mock.calls[0]?.[0].actions?.map((action) => action.kind),
		).toEqual(['choice', 'choice', 'reply', 'open']);
		expect(fixture.stdoutJson()).toMatchObject({
			status: 'answered',
			value: 'Use SQLite',
		});
	});

	it('exits 3 on deferred and leaves the message open', async () => {
		const { client, rescindMessage } = askingClient(actionResponse('deferred'));
		const fixture = createCommandFixture(client);
		expect(
			await runAsk(
				[
					'--title',
					'Which store should the hub use?',
					'--blocking',
					'deferrable',
				],
				fixture.context,
			),
		).toBe(EXIT.deferred);
		expect(fixture.stdoutJson()).toMatchObject({ status: 'deferred' });
		expect(rescindMessage).not.toHaveBeenCalled();
	});

	it.each(['dismissed', 'rescinded', 'expired', 'gone'] as ResponseStatus[])(
		'exits 4 when the message is %s',
		async (status) => {
			const { client } = askingClient(actionResponse(status));
			const fixture = createCommandFixture(client);
			expect(await runAsk(['--title', 'Allow it?'], fixture.context)).toBe(
				EXIT.noAnswer,
			);
			expect(fixture.stdoutJson()).toMatchObject({ status });
		},
	);

	it('exits 4 and leaves the message open when nobody can answer now', async () => {
		const { client, rescindMessage, getResponse } = askingClient(
			actionResponse('pending'),
			ingestResponse({ shouldWait: false }),
		);
		const fixture = createCommandFixture(client);
		expect(await runAsk(['--title', 'Allow it?'], fixture.context)).toBe(
			EXIT.noAnswer,
		);
		expect(fixture.stdoutJson()).toMatchObject({
			status: 'no_answer',
			reason: 'nobody_can_answer_now',
		});
		expect(fixture.stdoutJson()).not.toHaveProperty('messageWithdrawn');
		expect(rescindMessage).not.toHaveBeenCalled();
		expect(getResponse).not.toHaveBeenCalled();
	});

	it('exits 4 with reason muted, shows the warning, and leaves the message open', async () => {
		const warning = {
			code: 'muted',
			message:
				"Delivered to the inbox without a ping: topic 'Release 1.4' is muted",
		} as const;
		const { client, rescindMessage, getResponse } = askingClient(
			actionResponse('pending'),
			ingestResponse({ shouldWait: false, warnings: [warning] }),
		);
		const fixture = createCommandFixture(client);
		expect(await runAsk(['--title', 'Allow it?'], fixture.context)).toBe(
			EXIT.noAnswer,
		);
		expect(fixture.stdoutJson()).toMatchObject({
			messageId: MESSAGE_ID,
			status: 'no_answer',
			reason: 'muted',
		});
		expect(fixture.stdoutJson()).not.toHaveProperty('messageWithdrawn');
		expect(fixture.stderr()).toContain(`warning [muted]: ${warning.message}`);
		expect(rescindMessage).not.toHaveBeenCalled();
		expect(getResponse).not.toHaveBeenCalled();
	});

	it('supports action_required with steps, offering no approve or deny', async () => {
		const { client, postMessage } = askingClient(
			actionResponse('answered', { actionKind: 'done' }),
		);
		const code = await runAsk(
			[
				'--type',
				'action_required',
				'--title',
				'Sign in to npm so I can publish',
				'--step',
				'Sign in to npm',
			],
			createCommandFixture(client).context,
		);
		expect(code).toBe(EXIT.ok);
		expect(
			postMessage.mock.calls[0]?.[0].actions?.map((action) => action.kind),
		).toEqual(['open']);
	});

	it.each([['0'], ['abc'], ['99999']])(
		'rejects --timeout %s',
		async (timeout) => {
			const { client } = askingClient(actionResponse('pending'));
			expect(
				await runAsk(
					['--title', 'x', '--timeout', timeout],
					createCommandFixture(client).context,
				),
			).toBe(EXIT.usage);
		},
	);

	it('prints the hub warnings to stderr', async () => {
		const warning = {
			code: 'summary_looks_like_command',
			field: 'summary',
			message: 'Summary moved to the body.',
		} as const;
		const { client } = askingClient(
			actionResponse('answered', { actionKind: 'approve' }),
			ingestResponse({ warnings: [warning] }),
		);
		const fixture = createCommandFixture(client);
		await runAsk(['--title', 'Allow it?'], fixture.context);
		expect(fixture.stderr()).toContain('Summary moved to the body.');
	});
});
