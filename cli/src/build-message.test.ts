import { describe, expect, it } from 'vitest';
import {
	buildAnswerActions,
	buildMessage,
	type MessageDraft,
	type SenderContext,
	STEPS_REQUIRED_MESSAGE,
	toThreadReference,
	toTopicReference,
} from './build-message.ts';
import { isValidMessageJson } from './test-support/schema-validator.ts';

const ULID = '01JBM6R3K9P2W5X8Z1C4V7N0QD';

const CONTEXT: SenderContext = {
	agent: 'claude-code',
	displayName: 'Claude Code',
	conversationId: 'session-1',
	conversationTitle: 'Fix flaky ingest test',
	workingDirectory: '/Users/me/Developer/gentbox',
	now: () => new Date('2026-10-24T12:00:00Z'),
};

function build(draft: MessageDraft) {
	const built = buildMessage(draft, CONTEXT);
	if (!built.ok) throw new Error(built.error.join('; '));
	return built.value;
}

describe('buildMessage', () => {
	it('fills the sender, the conversation and the creation time, and satisfies the JSON Schema', () => {
		const message = build({
			title: 'Ingest auth accepts rotated tokens; 12 tests pass',
			type: 'task_completed',
		});
		expect(message).toMatchObject({
			version: 1,
			source: { agent: 'claude-code', displayName: 'Claude Code' },
			conversation: {
				id: 'session-1',
				title: 'Fix flaky ingest test',
				cwd: '/Users/me/Developer/gentbox',
			},
			type: 'task_completed',
			createdAt: '2026-10-24T12:00:00.000Z',
		});
		expect(isValidMessageJson(message)).toMatchObject({ isValid: true });
	});

	it('leaves undeclared fields out so the rules can fill them', () => {
		const message = build({ title: 'Something to know' });
		expect(JSON.parse(JSON.stringify(message))).not.toHaveProperty('type');
		expect(JSON.parse(JSON.stringify(message))).not.toHaveProperty('blocking');
	});

	it('requires steps for action_required and names the rule', () => {
		expect(
			buildMessage(
				{ title: 'Sign in to npm so I can publish', type: 'action_required' },
				CONTEXT,
			),
		).toEqual({
			ok: false,
			error: [STEPS_REQUIRED_MESSAGE],
		});
		expect(
			buildMessage(
				{ title: 'Sign in', type: 'action_required', steps: [] },
				CONTEXT,
			).ok,
		).toBe(false);
	});

	it('accepts action_required with steps', () => {
		const message = build({
			title: 'Sign in to npm so I can publish the CLI package',
			type: 'action_required',
			blocking: 'deferrable',
			urgent: true,
			steps: [
				{ text: 'Open a terminal in any folder' },
				{ text: 'Sign in to npm', detail: '```bash\nnpm login\n```' },
			],
		});
		expect(isValidMessageJson(message)).toMatchObject({ isValid: true });
	});

	it.each([
		[
			'a blocking task_completed',
			{ type: 'task_completed', blocking: 'blocking' },
			'/blocking',
		],
		['an urgent generic', { type: 'generic', urgent: true }, '/urgent'],
		[
			'a scope on a decision',
			{ type: 'decision_required', scope: 'goal' },
			'/scope',
		],
		['a title over 120 characters', { title: 'x'.repeat(121) }, '/title'],
		[
			'a bad correlation key',
			{ correlationKey: 'no spaces allowed' },
			'/correlationKey',
		],
	] as const)(
		'reports %s with the JSON pointer',
		(_name, overrides, pointer) => {
			const built = buildMessage({ title: 'A title', ...overrides }, CONTEXT);
			expect(built.ok).toBe(false);
			if (!built.ok) expect(built.error.join('\n')).toContain(pointer);
		},
	);

	it('uses the inline topic and thread shortcuts: a key creates, an id names', () => {
		const byKey = build({
			title: 'A title',
			topic: 'repo:gentbox',
			topicTitle: 'gentbox',
			thread: 'phase-3',
		});
		expect(byKey.topic).toEqual({ key: 'repo:gentbox', title: 'gentbox' });
		expect(byKey.thread).toEqual({ key: 'phase-3', title: undefined });
		const byId = build({
			title: 'A title',
			topic: ULID,
			thread: '01JBM9P3H4S5E6J7N8G9E0S1TQ',
		});
		expect(byId.topic).toEqual({ id: ULID });
		expect(byId.thread).toEqual({ id: '01JBM9P3H4S5E6J7N8G9E0S1TQ' });
		expect(isValidMessageJson(byKey)).toMatchObject({ isValid: true });
		expect(isValidMessageJson(byId)).toMatchObject({ isValid: true });
	});

	it('carries rescinds with its reason and keys', () => {
		const message = build({
			title: 'Plan replaced by a new one',
			rescinds: { reason: 'superseded', correlationKeys: ['publish-plan'] },
		});
		expect(isValidMessageJson(message)).toMatchObject({ isValid: true });
	});
});

describe('topic and thread references', () => {
	it('tell a ULID from a key', () => {
		expect(toTopicReference(ULID, 'ignored')).toEqual({ id: ULID });
		expect(toTopicReference('release-1.4', 'Release 1.4')).toEqual({
			key: 'release-1.4',
			title: 'Release 1.4',
		});
		expect(toTopicReference(undefined, 'x')).toBeUndefined();
		expect(toThreadReference(undefined, undefined)).toBeUndefined();
	});
});

describe('buildAnswerActions', () => {
	it('offers Approve and Deny for a decision without choices', () => {
		const actions = buildAnswerActions('decision_required', {
			choices: [],
			allowReply: false,
		});
		expect(actions.map((action) => action.kind)).toEqual([
			'approve',
			'deny',
			'open',
		]);
	});

	it('offers one button per choice, then Reply, then Open', () => {
		const actions = buildAnswerActions('decision_required', {
			choices: ['Use SQLite', 'Use Postgres'],
			allowReply: true,
		});
		expect(actions).toEqual([
			{
				id: 'choice-1',
				label: 'Use SQLite',
				kind: 'choice',
				value: 'Use SQLite',
			},
			{
				id: 'choice-2',
				label: 'Use Postgres',
				kind: 'choice',
				value: 'Use Postgres',
			},
			{ id: 'reply', label: 'Reply', kind: 'reply' },
			{ id: 'open', label: 'Open session', kind: 'open' },
		]);
	});

	it('leaves Done to the hub on action_required', () => {
		const actions = buildAnswerActions('action_required', {
			choices: ['ignored'],
			allowReply: false,
		});
		expect(actions.map((action) => action.kind)).toEqual(['open']);
	});
});
