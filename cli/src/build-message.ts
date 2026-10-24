import {
	type Action,
	type ActionStep,
	type BlockingLevel,
	isUlid,
	MESSAGE_SCHEMA_VERSION,
	type MessageType,
	type MessageV1,
	messageSchema,
	type RescindReason,
	type ThreadRef,
	type TopicRef,
} from './message-types.ts';
import { fail, type Result, succeed } from './result.ts';

/** Who is sending: fixed for the life of a CLI call or an MCP server. */
export type SenderContext = {
	agent: string;
	displayName: string | undefined;
	conversationId: string;
	conversationTitle: string | undefined;
	workingDirectory: string;
	now: () => Date;
};

/** What a caller chooses for one message; everything else is filled from the context. */
export type MessageDraft = {
	title: string;
	summary?: string | undefined;
	body?: string | undefined;
	type?: MessageType | undefined;
	urgent?: boolean | undefined;
	blocking?: BlockingLevel | undefined;
	scope?: 'task' | 'goal' | undefined;
	steps?: ActionStep[] | undefined;
	actions?: Action[] | undefined;
	awaitsResponse?: boolean | undefined;
	/** A topic id (ULID) or key; a key creates the topic on first use. */
	topic?: string | undefined;
	topicTitle?: string | undefined;
	/** A thread id (ULID) or key; a key creates the thread on first use. */
	thread?: string | undefined;
	threadTitle?: string | undefined;
	event?: string | undefined;
	correlationKey?: string | undefined;
	idempotencyKey?: string | undefined;
	sensitive?: boolean | undefined;
	expiresAt?: string | undefined;
	rescinds?: { reason: RescindReason; correlationKeys: string[] } | undefined;
};

export type AnswerChoices = {
	/** Labels of choice buttons; empty means Approve and Deny for a decision. */
	choices: string[];
	allowReply: boolean;
};

export const STEPS_REQUIRED_MESSAGE =
	"`steps` is required when `type` is `action_required`: ordered, plain instructions that say how to complete the action (commands and paths go in a step's `detail`)";

/**
 * Builds a message (absent optional fields stay `undefined` and are dropped by
 * JSON serialization) and checks it against the same rules as
 * docs/schema/message-v1.json, so a mistake is reported here in words rather
 * than as a 422 from the hub. `action_required` must carry `steps`.
 */
export function buildMessage(
	draft: MessageDraft,
	context: SenderContext,
): Result<MessageV1, string[]> {
	if (
		draft.type === 'action_required' &&
		(draft.steps === undefined || draft.steps.length === 0)
	) {
		return fail([STEPS_REQUIRED_MESSAGE]);
	}
	const topic = toTopicReference(draft.topic, draft.topicTitle);
	const thread = toThreadReference(draft.thread, draft.threadTitle);
	const candidate = {
		version: MESSAGE_SCHEMA_VERSION,
		idempotencyKey: draft.idempotencyKey,
		source: { agent: context.agent, displayName: context.displayName },
		conversation: {
			id: context.conversationId,
			title: context.conversationTitle,
			cwd: context.workingDirectory,
		},
		topic,
		thread,
		type: draft.type,
		urgent: draft.urgent,
		blocking: draft.blocking,
		event: draft.event,
		title: draft.title,
		summary: draft.summary,
		body: draft.body,
		steps: draft.steps,
		actions: draft.actions,
		scope: draft.scope,
		awaitsResponse: draft.awaitsResponse,
		correlationKey: draft.correlationKey,
		rescinds: draft.rescinds,
		sensitive: draft.sensitive,
		createdAt: context.now().toISOString(),
		expiresAt: draft.expiresAt,
	};
	const parsed = messageSchema.safeParse(candidate);
	if (!parsed.success) return fail(describeIssues(parsed.error.issues));
	return succeed(parsed.data);
}

/** `{ id }` for a ULID, else `{ key, title? }`: the inline form, which creates the topic on first use. */
export function toTopicReference(
	reference: string | undefined,
	title: string | undefined,
): TopicRef | undefined {
	if (reference === undefined) return undefined;
	if (isUlid(reference)) return { id: reference };
	return { key: reference, title };
}

/** `{ id }` for a ULID, else `{ key, title? }`: the inline form, which creates the thread on first use. */
export function toThreadReference(
	reference: string | undefined,
	title: string | undefined,
): ThreadRef | undefined {
	if (reference === undefined) return undefined;
	if (isUlid(reference)) return { id: reference };
	return { key: reference, title };
}

/**
 * The answer buttons of an `ask`. A decision with no choices offers Approve
 * and Deny; with choices, one button per choice. `allowReply` adds a free-text
 * Reply (and, with no choices, replaces Approve and Deny: a question that is
 * not yes or no). The open button comes last. An `action_required` message
 * gets only Reply and Open: the hub adds Done itself.
 */
export function buildAnswerActions(
	type: MessageType,
	answers: AnswerChoices,
): Action[] {
	const answerButtons: Action[] = [];
	if (type === 'decision_required') {
		const isApproval = answers.choices.length === 0 && !answers.allowReply;
		if (isApproval) {
			answerButtons.push(
				{ id: 'approve', label: 'Approve', kind: 'approve' },
				{ id: 'deny', label: 'Deny', kind: 'deny', destructive: true },
			);
		}
		answerButtons.push(
			...answers.choices.map(
				(label, index): Action => ({
					id: `choice-${index + 1}`,
					label,
					kind: 'choice',
					value: label,
				}),
			),
		);
	}
	if (answers.allowReply)
		answerButtons.push({ id: 'reply', label: 'Reply', kind: 'reply' });
	return [
		...answerButtons,
		{ id: 'open', label: 'Open session', kind: 'open' },
	];
}

export function describeIssues(
	issues: ReadonlyArray<{ path: PropertyKey[]; message: string }>,
): string[] {
	return issues.map((issue) => {
		const pointer = issue.path.map((segment) => `/${String(segment)}`).join('');
		return pointer === '' ? issue.message : `${pointer}: ${issue.message}`;
	});
}
