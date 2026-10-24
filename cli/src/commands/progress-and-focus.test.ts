import { describe, expect, it, vi } from 'vitest';
import type { HubClient } from '../hub-client.ts';
import { succeed } from '../result.ts';
import { createCommandFixture } from '../test-support/command-fixture.ts';
import { MESSAGE_ID } from '../test-support/hub-fixtures.ts';
import { stubHubClient } from '../test-support/stub-hub-client.ts';
import { EXIT } from './command-context.ts';
import { runFocus } from './focus.ts';
import { runProgress } from './progress.ts';

describe('gentbox progress', () => {
	it('reports progress and prints what was withdrawn', async () => {
		const reportProgress = vi.fn<HubClient['reportProgress']>(async () =>
			succeed([MESSAGE_ID]),
		);
		const fixture = createCommandFixture(stubHubClient({ reportProgress }));
		expect(await runProgress([], fixture.context)).toBe(EXIT.ok);
		expect(fixture.stdoutJson()).toEqual({ rescinded: [MESSAGE_ID] });
		expect(await runProgress(['extra'], fixture.context)).toBe(EXIT.usage);
	});
});

describe('gentbox focus', () => {
	it('sets focus on and off', async () => {
		const setFocus = vi.fn<HubClient['setFocus']>(async () =>
			succeed(undefined),
		);
		const fixture = createCommandFixture(stubHubClient({ setFocus }));
		expect(await runFocus(['on'], fixture.context)).toBe(EXIT.ok);
		expect(await runFocus(['off'], fixture.context)).toBe(EXIT.ok);
		expect(setFocus.mock.calls.map((call) => call[0])).toEqual([true, false]);
	});

	it.each([[[]], [['maybe']], [['on', 'off']]])('rejects %j', async (args) => {
		expect(
			await runFocus(args, createCommandFixture(stubHubClient()).context),
		).toBe(EXIT.usage);
	});
});
