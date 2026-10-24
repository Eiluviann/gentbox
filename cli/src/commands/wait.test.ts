import { describe, expect, it, vi } from 'vitest';
import type { HubClient } from '../hub-client.ts';
import { succeed } from '../result.ts';
import { createCommandFixture } from '../test-support/command-fixture.ts';
import { actionResponse, MESSAGE_ID } from '../test-support/hub-fixtures.ts';
import { stubHubClient } from '../test-support/stub-hub-client.ts';
import { EXIT } from './command-context.ts';
import { runWait } from './wait.ts';

describe('gentbox wait', () => {
	it('prints the answer to a message sent earlier', async () => {
		const getResponse = vi.fn<HubClient['getResponse']>(async () =>
			succeed(actionResponse('answered', { actionKind: 'done' })),
		);
		const fixture = createCommandFixture(stubHubClient({ getResponse }));
		expect(
			await runWait([MESSAGE_ID, '--timeout', '30'], fixture.context),
		).toBe(EXIT.ok);
		expect(getResponse).toHaveBeenCalledWith(MESSAGE_ID, 30, undefined);
		expect(fixture.stdoutJson()).toMatchObject({
			status: 'answered',
			actionKind: 'done',
		});
	});

	it('does not withdraw the message when the answer is a dismissal', async () => {
		const fixture = createCommandFixture(
			stubHubClient({
				getResponse: async () => succeed(actionResponse('dismissed')),
			}),
		);
		expect(await runWait([MESSAGE_ID], fixture.context)).toBe(EXIT.noAnswer);
	});

	it.each([[[]], [['not-an-id']], [[MESSAGE_ID, MESSAGE_ID]]])(
		'rejects %j',
		async (args) => {
			expect(
				await runWait(args, createCommandFixture(stubHubClient()).context),
			).toBe(EXIT.usage);
		},
	);
});
