import { describe, expect, it } from 'vitest';
import { readSenderContext } from './sender-context.ts';
import { environmentOf } from './test-support/environment-of.ts';

const DEFAULTS = { agent: 'claude-code', conversationId: () => 'mcp-1' };

describe('readSenderContext', () => {
	it('takes the focus target for "Open session" from the environment', () => {
		const sender = readSenderContext(
			environmentOf([
				'GENTBOX_FOCUS_BUNDLE_ID',
				'com.anthropic.claudefordesktop',
			]),
			DEFAULTS,
			'/work',
		);
		expect(sender.focus).toEqual({
			bundleId: 'com.anthropic.claudefordesktop',
			url: undefined,
		});
	});

	it('has no focus target by default', () => {
		expect(
			readSenderContext(environmentOf(), DEFAULTS, '/work').focus,
		).toBeUndefined();
	});

	it('refuses a malformed agent or conversation id', () => {
		expect(() =>
			readSenderContext(
				environmentOf(['GENTBOX_AGENT', 'Not Valid']),
				DEFAULTS,
				'/work',
			),
		).toThrow(/GENTBOX_AGENT/);
		expect(() =>
			readSenderContext(
				environmentOf(['GENTBOX_CONVERSATION', 'has space']),
				DEFAULTS,
				'/work',
			),
		).toThrow(/GENTBOX_CONVERSATION/);
	});
});
