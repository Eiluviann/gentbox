import { z } from 'zod';

// Written against docs/schema/message-v1.json; message-types.test.ts checks
// every file in docs/schema/examples against both this file and the schema.

/** Version of the ingest message schema the CLI speaks (`docs/schema/message-v1.json`). */
export const MESSAGE_SCHEMA_VERSION = 1;

const ULID_PATTERN = /^[0-9A-HJKMNP-TV-Z]{26}$/;

const ulidSchema = z.string().regex(ULID_PATTERN);
const tokenSchema = z.string().regex(/^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/);
const topicKeySchema = z.string().regex(/^[a-z0-9][a-z0-9._:/-]{0,95}$/);
const threadKeySchema = z.string().regex(/^[a-z0-9][a-z0-9._:/-]{0,95}$/);
const groupKindSchema = z.string().regex(/^[a-z0-9][a-z0-9_-]{0,31}$/);
const dateTimeSchema = z.iso.datetime({ offset: true });

export const messageTypeSchema = z.enum([
	'decision_required',
	'action_required',
	'task_completed',
	'generic',
]);
export type MessageType = z.infer<typeof messageTypeSchema>;

export const blockingLevelSchema = z.enum([
	'non_blocking',
	'deferrable',
	'blocking',
]);
export type BlockingLevel = z.infer<typeof blockingLevelSchema>;

/** The reasons a sender names when it withdraws its own message. */
export const rescindReasonSchema = z.enum([
	'resolved_elsewhere',
	'obsolete',
	'superseded',
]);
export type RescindReason = z.infer<typeof rescindReasonSchema>;

/**
 * The reasons the hub reports on a closed message: a sender's own, or
 * `handled_in_session`, the hub's clean-up when a completion or progress call
 * showed the item was handled in the session. A sender never sends the latter,
 * and it is not a withdrawal.
 */
export const KNOWN_RECORDED_RESCIND_REASONS = [
	...rescindReasonSchema.options,
	'handled_in_session',
] as const;

/** Read from the hub, so an open string: a newer hub's reason still parses. */
export const recordedRescindReasonSchema = z.string();
export type RecordedRescindReason = string;

export const rescindRequestSchema = z.strictObject({
	reason: rescindReasonSchema,
	correlationKeys: z.array(tokenSchema).min(1).max(32).optional(),
});
export type RescindRequest = z.infer<typeof rescindRequestSchema>;

const rescindsFieldSchema = z.strictObject({
	reason: rescindReasonSchema,
	correlationKeys: z.array(tokenSchema).min(1).max(32),
});

export const topicStyleSchema = z.strictObject({
	symbol: z
		.string()
		.regex(/^[a-z0-9]+(\.[a-z0-9]+)*$/)
		.max(64)
		.optional(),
	color: z
		.enum([
			'blue',
			'purple',
			'pink',
			'red',
			'orange',
			'yellow',
			'green',
			'teal',
			'indigo',
			'gray',
		])
		.optional(),
});
export type TopicStyle = z.infer<typeof topicStyleSchema>;

const topicRefByIdSchema = z.strictObject({ id: ulidSchema });
const topicRefByKeySchema = z.strictObject({
	key: topicKeySchema,
	title: z.string().min(1).max(80).optional(),
	summary: z.string().max(400).optional(),
	style: topicStyleSchema.optional(),
	kind: groupKindSchema.optional(),
});
export const topicRefSchema = z.union([
	topicRefByIdSchema,
	topicRefByKeySchema,
]);
export type TopicRef = z.infer<typeof topicRefSchema>;

const threadRefByIdSchema = z.strictObject({ id: ulidSchema });
const threadRefByKeySchema = z.strictObject({
	key: threadKeySchema,
	title: z.string().min(1).max(80).optional(),
	summary: z.string().max(400).optional(),
	kind: groupKindSchema.optional(),
});
export const threadRefSchema = z.union([
	threadRefByIdSchema,
	threadRefByKeySchema,
]);
export type ThreadRef = z.infer<typeof threadRefSchema>;

export const sourceSchema = z.strictObject({
	agent: z.string().regex(/^[a-z0-9][a-z0-9-]{0,63}$/),
	displayName: z.string().max(64).optional(),
	version: z.string().max(64).optional(),
	host: z.string().max(255).optional(),
});
export type Source = z.infer<typeof sourceSchema>;

export const focusTargetSchema = z.strictObject({
	url: z.string().max(2048).optional(),
	bundleId: z.string().max(255).optional(),
	terminal: z
		.strictObject({
			tty: z
				.string()
				.regex(/^\/dev\/[A-Za-z0-9/]+$/)
				.optional(),
			tmuxPane: z
				.string()
				.regex(/^%[0-9]+$/)
				.optional(),
			pid: z.number().int().min(1).optional(),
		})
		.optional(),
});

export const conversationSchema = z.strictObject({
	id: tokenSchema,
	title: z.string().max(120).optional(),
	cwd: z.string().max(4096).optional(),
	focus: focusTargetSchema.optional(),
});
export type Conversation = z.infer<typeof conversationSchema>;

export const actionKindSchema = z.enum([
	'approve',
	'deny',
	'choice',
	'reply',
	'done',
	'defer',
	'open',
	'snooze',
]);
export type ActionKind = z.infer<typeof actionKindSchema>;

export const actionSchema = z
	.strictObject({
		id: z.string().regex(/^[a-z0-9][a-z0-9_-]{0,31}$/),
		label: z.string().min(1).max(40),
		kind: actionKindSchema,
		value: z.string().max(256).optional(),
		placeholder: z.string().max(80).optional(),
		destructive: z.boolean().optional(),
	})
	.superRefine((action, context) => {
		if (action.kind === 'choice' && action.value === undefined) {
			context.addIssue({
				code: 'custom',
				message: 'a choice action needs a value',
				path: ['value'],
			});
		}
		if (
			action.kind === 'snooze' &&
			action.value !== undefined &&
			!/^[1-9][0-9]{0,3}$/.test(action.value)
		) {
			context.addIssue({
				code: 'custom',
				message: 'a snooze value is minutes as a decimal string',
				path: ['value'],
			});
		}
	});
export type Action = z.infer<typeof actionSchema>;

export const actionStepSchema = z.strictObject({
	text: z.string().min(1).max(200),
	detail: z.string().max(2000).optional(),
	link: z
		.strictObject({
			url: z
				.string()
				.regex(
					/^(https|claude-cli|claude|vscode|cursor|x-apple\.systempreferences):/,
				)
				.max(2048),
			label: z.string().min(1).max(60).optional(),
		})
		.optional(),
});
export type ActionStep = z.infer<typeof actionStepSchema>;

const metadataSchema = z
	.record(
		z.string().regex(/^[A-Za-z][A-Za-z0-9_]{0,63}$/),
		z.union([z.string().max(1024), z.number(), z.boolean()]),
	)
	.refine((metadata) => Object.keys(metadata).length <= 32, {
		message: 'at most 32 metadata properties',
	});

export const messageSchema = z
	.strictObject({
		version: z.literal(MESSAGE_SCHEMA_VERSION),
		idempotencyKey: tokenSchema.optional(),
		source: sourceSchema,
		conversation: conversationSchema,
		topic: topicRefSchema.optional(),
		thread: threadRefSchema.optional(),
		type: messageTypeSchema.optional(),
		urgent: z.boolean().optional(),
		blocking: blockingLevelSchema.optional(),
		event: z
			.string()
			.regex(/^[a-z0-9][a-z0-9_.-]{0,127}$/)
			.optional(),
		title: z.string().min(1).max(120),
		summary: z.string().max(400).optional(),
		body: z.string().max(65536).optional(),
		steps: z.array(actionStepSchema).min(1).max(12).optional(),
		actions: z.array(actionSchema).max(6).optional(),
		scope: z.enum(['task', 'goal']).optional(),
		awaitsResponse: z.boolean().optional(),
		correlationKey: tokenSchema.optional(),
		rescinds: rescindsFieldSchema.optional(),
		sensitive: z.boolean().optional(),
		createdAt: dateTimeSchema.optional(),
		expiresAt: dateTimeSchema.optional(),
		metadata: metadataSchema.optional(),
	})
	.superRefine((message, context) => {
		const neverBlocks =
			message.type === 'task_completed' || message.type === 'generic';
		if (
			neverBlocks &&
			message.blocking !== undefined &&
			message.blocking !== 'non_blocking'
		) {
			context.addIssue({
				code: 'custom',
				message: '`task_completed` and `generic` never block',
				path: ['blocking'],
			});
		}
		if (message.type === 'generic' && message.urgent === true) {
			context.addIssue({
				code: 'custom',
				message: '`generic` is never urgent',
				path: ['urgent'],
			});
		}
		if (message.scope !== undefined && message.type !== 'task_completed') {
			context.addIssue({
				code: 'custom',
				message: '`scope` belongs to `task_completed`',
				path: ['scope'],
			});
		}
		if (message.steps !== undefined && message.type !== 'action_required') {
			context.addIssue({
				code: 'custom',
				message: '`steps` belong to `action_required`',
				path: ['steps'],
			});
		}
	});
export type MessageV1 = z.infer<typeof messageSchema>;

export const closeRequestSchema = z.strictObject({
	rescindOpenMessages: z.boolean().optional(),
	reason: rescindReasonSchema.optional(),
});
export type CloseRequest = z.infer<typeof closeRequestSchema>;

export const topicCreateSchema = z.strictObject({
	key: topicKeySchema,
	title: z.string().min(1).max(80),
	summary: z.string().max(400).optional(),
	kind: groupKindSchema.optional(),
	style: topicStyleSchema.optional(),
	conversation: z.strictObject({ id: tokenSchema }).optional(),
});
export type TopicCreate = z.infer<typeof topicCreateSchema>;

export const threadCreateSchema = z.strictObject({
	topic: topicRefSchema.optional(),
	key: threadKeySchema,
	title: z.string().min(1).max(80),
	summary: z.string().max(400).optional(),
	kind: groupKindSchema.optional(),
	conversation: z.strictObject({ id: tokenSchema }).optional(),
});
export type ThreadCreate = z.infer<typeof threadCreateSchema>;

export const threadMoveSchema = z.strictObject({ topic: topicRefSchema });
export type ThreadMove = z.infer<typeof threadMoveSchema>;

// Everything below up to `rescindedIdsSchema` is read from the hub, so it is
// lenient: unknown fields are dropped and warning codes are open, because the
// hub may add optional fields within v1 (architecture §8, Versioning) and a
// strict reader shipped in a plugin could not be fixed in place. What the
// client sends stays strict.

const topicStyleReadSchema = z.object({
	symbol: z.string().optional(),
	color: z.string().optional(),
});

const createdBySchema = z.object({
	agent: z.string(),
	conversationId: z.string().optional(),
});

export const topicRecordSchema = z.object({
	id: ulidSchema,
	key: topicKeySchema.optional(),
	title: z.string(),
	summary: z.string().optional(),
	kind: groupKindSchema.optional(),
	implicit: z.boolean(),
	style: topicStyleReadSchema.optional(),
	status: z.enum(['open', 'closed']),
	createdBy: createdBySchema,
	createdAt: dateTimeSchema,
	updatedAt: dateTimeSchema,
	closedAt: dateTimeSchema.optional(),
	openMessages: z.number().int().min(0).optional(),
	openThreads: z.number().int().min(0).optional(),
});
export type TopicRecord = z.infer<typeof topicRecordSchema>;

export const threadRecordSchema = z.object({
	id: ulidSchema,
	topicId: ulidSchema,
	key: threadKeySchema.optional(),
	title: z.string(),
	summary: z.string().optional(),
	kind: groupKindSchema.optional(),
	implicit: z.boolean(),
	status: z.enum(['open', 'closed']),
	defaultFor: z
		.object({ agent: z.string(), conversationId: z.string() })
		.optional(),
	createdBy: createdBySchema,
	createdAt: dateTimeSchema,
	updatedAt: dateTimeSchema,
	closedAt: dateTimeSchema.optional(),
	openMessages: z.number().int().min(0).optional(),
	conversations: z.number().int().min(0).optional(),
});
export type ThreadRecord = z.infer<typeof threadRecordSchema>;

/** Warning codes the hub sends today; the type is open, so a newer hub's code still parses. */
export const KNOWN_WARNING_CODES = [
	'title_looks_like_command',
	'summary_looks_like_command',
	'conversation_title_ignored',
	'normalized',
	'steps_missing',
	'step_rewritten',
	'secret_redacted',
	'muted',
] as const;

export const ingestWarningSchema = z.object({
	code: z.string(),
	field: z.string().optional(),
	message: z.string(),
});
export type IngestWarning = z.infer<typeof ingestWarningSchema>;

export const ingestResponseSchema = z.object({
	id: ulidSchema,
	type: messageTypeSchema,
	urgent: z.boolean(),
	blocking: blockingLevelSchema,
	fieldSources: z.record(z.string(), z.string()).optional(),
	topicId: ulidSchema,
	threadId: ulidSchema,
	state: z.enum([
		'open',
		'acknowledged',
		'answered',
		'dismissed',
		'rescinded',
		'expired',
	]),
	shouldWait: z.boolean(),
	warnings: z.array(ingestWarningSchema).optional(),
});
export type IngestResponse = z.infer<typeof ingestResponseSchema>;

export const responseStatusSchema = z.enum([
	'pending',
	'answered',
	'deferred',
	'dismissed',
	'rescinded',
	'expired',
	'gone',
]);
export type ResponseStatus = z.infer<typeof responseStatusSchema>;

export const actionResponseSchema = z.object({
	messageId: ulidSchema,
	status: responseStatusSchema,
	rescindReason: recordedRescindReasonSchema.optional(),
	actionId: z.string().optional(),
	actionKind: z.enum(['approve', 'deny', 'choice', 'reply', 'done']).optional(),
	value: z.string().max(8192).optional(),
	answeredVia: z.enum(['mac', 'phone', 'watch', 'cli']).optional(),
	answeredAt: dateTimeSchema.optional(),
});
export type ActionResponse = z.infer<typeof actionResponseSchema>;

/** `POST /v1/hello` answer (core `HelloResponse`); not part of the JSON Schema. */
export const helloResponseSchema = z.object({
	proof: z.string(),
	hub: z.string(),
	version: z.string(),
	schemaVersions: z.array(z.number().int()),
});
export type HelloResponse = z.infer<typeof helloResponseSchema>;

/** Error body of every non-2xx hub answer (`ErrorBody` in `gentbox-ingest`). */
export const errorBodySchema = z.object({
	code: z.string(),
	message: z.string(),
	pointer: z.string().optional(),
	state: z.string().optional(),
	/** With `has_open_messages`: how many open messages stop a close. */
	openMessages: z.number().int().min(0).optional(),
	/** An older spelling of `openMessages`. */
	count: z.number().int().min(0).optional(),
});
export type ErrorBody = z.infer<typeof errorBodySchema>;

/** `{ "rescinded": [ids] }`: answer of the conversation, topic and thread rescind routes. */
export const rescindedIdsSchema = z.object({
	rescinded: z.array(ulidSchema),
});

export const topicListSchema = z.object({
	topics: z.array(topicRecordSchema),
});
export const threadListSchema = z.object({
	threads: z.array(threadRecordSchema),
});

/** Whether a string is a hub-assigned id (a ULID) rather than an agent-chosen key. */
export function isUlid(text: string): boolean {
	return ULID_PATTERN.test(text);
}
