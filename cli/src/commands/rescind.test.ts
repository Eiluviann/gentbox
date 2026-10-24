import { describe, expect, it, vi } from 'vitest';
import type { HubClient } from '../hub-client.ts';
import { succeed } from '../result.ts';
import { createCommandFixture } from '../test-support/command-fixture.ts';
import { actionResponse, MESSAGE_ID } from '../test-support/hub-fixtures.ts';
import { stubHubClient } from '../test-support/stub-hub-client.ts';
import { EXIT } from './command-context.ts';
import { runRescind } from './rescind.ts';

describe('gentbox rescind', () => {
	it('withdraws one message, as resolved_elsewhere unless told otherwise', async () => {
		const rescindMessage = vi.fn<HubClient['rescindMessage']>(async () =>
			succeed(actionResponse('rescinded', { rescindReason: 'obsolete' })),
		);
		const client = stubHubClient({ rescindMessage });
		const fixture = createCommandFixture(client);
		expect(await runRescind([MESSAGE_ID], fixture.context)).toBe(EXIT.ok);
		expect(
			await runRescind([MESSAGE_ID, '--reason', 'obsolete'], fixture.context),
		).toBe(EXIT.ok);
		expect(rescindMessage.mock.calls.map((call) => call[1])).toEqual([
			'resolved_elsewhere',
			'obsolete',
		]);
	});

	it('withdraws by correlation keys on the conversation route, as superseded by default', async () => {
		const rescindByCorrelationKeys = vi.fn<
			HubClient['rescindByCorrelationKeys']
		>(async () => succeed([MESSAGE_ID]));
		const fixture = createCommandFixture(
			stubHubClient({ rescindByCorrelationKeys }),
		);
		expect(
			await runRescind(['--key', 'plan-a', '--key', 'plan-b'], fixture.context),
		).toBe(EXIT.ok);
		expect(rescindByCorrelationKeys).toHaveBeenCalledWith({
			reason: 'superseded',
			correlationKeys: ['plan-a', 'plan-b'],
		});
		expect(fixture.stdoutJson()).toEqual({ rescinded: [MESSAGE_ID] });
	});

	it.each([
		[[]],
		[[MESSAGE_ID, '--key', 'a']],
		[['nope']],
		[[MESSAGE_ID, '--reason', 'because']],
	])('rejects %j', async (args) => {
		expect(
			await runRescind(args, createCommandFixture(stubHubClient()).context),
		).toBe(EXIT.usage);
	});
});
