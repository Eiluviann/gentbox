import { z } from 'zod';
import {
	actionStepSchema,
	blockingLevelSchema,
	messageTypeSchema,
	rescindReasonSchema,
	topicStyleSchema,
} from './message-types.ts';

/** `type: "object"` JSON Schema, which is all MCP asks of a tool's `inputSchema`. */
export type ToolInputJsonSchema = {
	type: 'object';
	properties?: Record<string, unknown>;
	required?: string[];
	allOf?: unknown[];
	[key: string]: unknown;
};

export const TITLE_RULE =
	'`title` is one line in plain words saying what you ask or did. It is never a command, path, URL, code or raw tool input: those go in `body`, where they show as code boxes. ' +
	'Good: "Allow Bash to run the ingest tests?". Bad: "cargo test -p gentbox-ingest -- --nocapture". ' +
	'The hub rewrites a title that looks like a command and returns a warning; write it right the first time.';

export const STEPS_RULE =
	'When `type` is `action_required`, `steps` is required: an ordered plan of plain instructions that tells the user how to do it, each `{ text, detail?, link? }` (1 to 12). ' +
	"Commands and paths go in a step's `detail`, never its `text`. " +
	'Example: [{"text":"Open a terminal in any folder"},{"text":"Sign in to npm","detail":"```bash\\nnpm login\\n```"}].';

const GROUPING_RULE =
	"`topic` and `thread` file the message under a project or phase: pass a key you choose (created on first use, named by `topicTitle` / `threadTitle`) or an id from create_topic / create_thread. Without them it lands in this conversation's default place.";

const ulidSchema = z.string().regex(/^[0-9A-HJKMNP-TV-Z]{26}$/);
const titleSchema = z
	.string()
	.min(1)
	.max(120)
	.describe('One line in plain words; never a command, path or URL.');
const summarySchema = z
	.string()
	.max(400)
	.optional()
	.describe(
		'One or two plain sentences: what is needed or what was done. Same rule as the title.',
	);
const bodySchema = z
	.string()
	.max(65536)
	.optional()
	.describe('Markdown. The place for commands, paths, URLs and raw input.');
const keySchema = z.string().max(96).optional();
const stepsSchema = z
	.array(actionStepSchema)
	.min(1)
	.max(12)
	.optional()
	.describe(
		'The action plan; required when type is action_required, not allowed otherwise.',
	);
const correlationKeySchema = z
	.string()
	.max(128)
	.optional()
	.describe(
		'A new message with the key of your open message replaces it in place instead of pinging again.',
	);

const groupingFields = {
	topic: keySchema.describe('Topic key (created on first use) or id.'),
	topicTitle: z
		.string()
		.min(1)
		.max(80)
		.optional()
		.describe('Plain-words name for a new topic.'),
	thread: keySchema.describe('Thread key (created on first use) or id.'),
	threadTitle: z
		.string()
		.min(1)
		.max(80)
		.optional()
		.describe('Plain-words name for a new thread.'),
};

function requireStepsForActions(
	input: { type?: string | undefined; steps?: unknown[] | undefined },
	context: z.RefinementCtx,
): void {
	if (input.type === 'action_required' && input.steps === undefined) {
		context.addIssue({
			code: 'custom',
			message: '`steps` is required when `type` is `action_required`',
			path: ['steps'],
		});
	}
	if (input.type !== 'action_required' && input.steps !== undefined) {
		context.addIssue({
			code: 'custom',
			message: '`steps` belongs to `action_required`',
			path: ['steps'],
		});
	}
}

export const notifyInputSchema = z
	.strictObject({
		type: messageTypeSchema.describe(
			'decision_required: you need an answer (prefer `ask`). action_required: the user must do something outside the conversation, then say Done. task_completed: work finished. generic: FYI, inbox only.',
		),
		title: titleSchema,
		summary: summarySchema,
		body: bodySchema,
		urgent: z
			.boolean()
			.optional()
			.describe('Only when the user must see it now. Never for generic.'),
		blocking: blockingLevelSchema
			.optional()
			.describe(
				'non_blocking (default): you keep working. deferrable: you paused but could carry on with other work. blocking: you cannot continue. Always non_blocking for task_completed and generic.',
			),
		scope: z
			.enum(['task', 'goal'])
			.optional()
			.describe(
				'task_completed only: goal when the overall objective is reached.',
			),
		steps: stepsSchema,
		correlationKey: correlationKeySchema,
		...groupingFields,
	})
	.superRefine(requireStepsForActions);
export type NotifyInput = z.infer<typeof notifyInputSchema>;

export const askInputSchema = z
	.strictObject({
		type: z
			.enum(['decision_required', 'action_required'])
			.optional()
			.describe(
				'decision_required (default): you need an answer. action_required: the user does something outside the conversation, then taps Done; requires steps.',
			),
		title: titleSchema,
		summary: summarySchema,
		body: bodySchema,
		urgent: z
			.boolean()
			.optional()
			.describe('Only when the user must see it now.'),
		blocking: z
			.enum(['blocking', 'deferrable'])
			.optional()
			.describe(
				'blocking (default): you cannot continue without the answer. deferrable: you can carry on with other work if the user answers Defer.',
			),
		choices: z
			.array(z.string().min(1).max(40))
			.min(1)
			.max(4)
			.optional()
			.describe(
				'Answer buttons; the answer is the chosen label. Without choices a decision offers Approve and Deny.',
			),
		allowReply: z
			.boolean()
			.optional()
			.describe(
				'Add a free-text Reply. With no choices it replaces Approve and Deny, for a question that is not yes or no.',
			),
		timeoutSeconds: z
			.number()
			.int()
			.min(1)
			.max(3600)
			.optional()
			.describe(
				'How long to wait for the answer (default 300). On expiry the message is withdrawn.',
			),
		steps: stepsSchema,
		correlationKey: correlationKeySchema,
		...groupingFields,
	})
	.superRefine((input, context) =>
		requireStepsForActions(
			{ type: input.type ?? 'decision_required', steps: input.steps },
			context,
		),
	);
export type AskInput = z.infer<typeof askInputSchema>;

export const waitInputSchema = z.strictObject({
	messageId: ulidSchema.describe(
		'The messageId `ask` returned when the user deferred (or `notify` returned).',
	),
	timeoutSeconds: z
		.number()
		.int()
		.min(1)
		.max(3600)
		.optional()
		.describe(
			'How long to wait (default 300). The message stays open if nothing arrives.',
		),
});
export type WaitInput = z.infer<typeof waitInputSchema>;

export const rescindInputSchema = z
	.strictObject({
		messageId: ulidSchema.optional().describe('The id notify or ask returned.'),
		correlationKeys: z
			.array(z.string())
			.min(1)
			.max(32)
			.optional()
			.describe(
				'Withdraw every open message of this conversation with these correlation keys.',
			),
		reason: rescindReasonSchema
			.optional()
			.describe(
				'resolved_elsewhere (default): handled in the session. obsolete: no longer needed. superseded: replaced.',
			),
	})
	.refine(
		(input) =>
			(input.messageId === undefined) !== (input.correlationKeys === undefined),
		{
			message: 'give either messageId or correlationKeys',
		},
	);
export type RescindInput = z.infer<typeof rescindInputSchema>;

export const createTopicInputSchema = z.strictObject({
	key: z
		.string()
		.min(1)
		.max(96)
		.describe(
			'Stable key you choose, e.g. "release-1.4" or "repo:gentbox"; creating an existing key returns it.',
		),
	title: z
		.string()
		.min(1)
		.max(80)
		.describe(
			'Plain-words name. Good: "Release 1.4". Bad: "npm publish --tag next". Never a command or path.',
		),
	summary: z.string().max(400).optional(),
	kind: z
		.string()
		.max(32)
		.optional()
		.describe('Free-form hint: project, category, other.'),
	style: topicStyleSchema
		.optional()
		.describe('Optional SF Symbol name and colour hints.'),
});
export type CreateTopicInput = z.infer<typeof createTopicInputSchema>;

export const closeTopicInputSchema = z.strictObject({
	topic: z.string().min(1).describe('Topic key or id.'),
	rescindOpenMessages: z
		.boolean()
		.optional()
		.describe('Also withdraw every open message in it.'),
	reason: rescindReasonSchema
		.optional()
		.describe('With rescindOpenMessages; default obsolete.'),
});
export type CloseTopicInput = z.infer<typeof closeTopicInputSchema>;

export const createThreadInputSchema = z.strictObject({
	key: z
		.string()
		.min(1)
		.max(96)
		.describe('Stable key you choose within the topic, e.g. "phase-3-ingest".'),
	title: z
		.string()
		.min(1)
		.max(80)
		.describe(
			'Plain-words name. Good: "Phase 3 ingest". Bad: "npm publish --tag next". Never a command or path.',
		),
	summary: z.string().max(400).optional(),
	kind: z
		.string()
		.max(32)
		.optional()
		.describe('Free-form hint: version, phase, conversation, other.'),
	topic: z
		.string()
		.optional()
		.describe("Topic key or id; default is the conversation's current topic."),
	topicTitle: z
		.string()
		.min(1)
		.max(80)
		.optional()
		.describe('Plain-words name if the topic key is new.'),
	standsForCurrentConversation: z
		.boolean()
		.optional()
		.describe(
			"Make the thread stand for this conversation: it takes over the conversation's automatic thread in the topic.",
		),
});
export type CreateThreadInput = z.infer<typeof createThreadInputSchema>;

export const closeThreadInputSchema = z.strictObject({
	thread: z
		.string()
		.min(1)
		.describe('Thread id, or its key together with `topic`.'),
	topic: z
		.string()
		.optional()
		.describe('Topic key or id; needed when `thread` is a key.'),
	rescindOpenMessages: z
		.boolean()
		.optional()
		.describe('Also withdraw every open message in it.'),
	reason: rescindReasonSchema
		.optional()
		.describe('With rescindOpenMessages; default obsolete.'),
});
export type CloseThreadInput = z.infer<typeof closeThreadInputSchema>;

export type ToolName =
	| 'notify'
	| 'ask'
	| 'wait'
	| 'rescind'
	| 'create_topic'
	| 'close_topic'
	| 'create_thread'
	| 'close_thread';

export type ToolSpec = {
	name: ToolName;
	description: string;
	schema: z.ZodType;
	inputSchema: ToolInputJsonSchema;
};

const STEPS_CONDITION = {
	if: {
		properties: { type: { const: 'action_required' } },
		required: ['type'],
	},
	// biome-ignore lint/suspicious/noThenProperty: `then` is a JSON Schema keyword
	then: { required: ['steps'] },
};
/** Includes an absent `type`: `ask` then means decision_required, which has no steps. */
const NO_STEPS_UNLESS_ACTION = {
	if: { not: STEPS_CONDITION.if },
	// biome-ignore lint/suspicious/noThenProperty: `then` is a JSON Schema keyword
	then: { not: { required: ['steps'] } },
};

/** The JSON Schema of a zod object, as MCP wants it (`type: "object"`). */
function toInputSchema(
	schema: z.ZodType,
	extra: Partial<ToolInputJsonSchema> = {},
): ToolInputJsonSchema {
	const { $schema: _dialect, ...json } = z.toJSONSchema(schema, {
		io: 'input',
	});
	return { ...json, type: 'object', ...extra };
}

export const TOOL_SPECS: ToolSpec[] = [
	{
		name: 'notify',
		description: [
			"Send a notification to the user's Gentbox inbox and return at once. It does not wait for an answer; use `ask` when you need one.",
			'Pick `type` deliberately: action_required when the user must do something themselves, task_completed when work finished, generic for anything worth keeping for reference.',
			TITLE_RULE,
			STEPS_RULE,
			GROUPING_RULE,
			"The result repeats the hub's warnings (title rewritten, steps missing, secrets redacted) verbatim; heed them.",
		].join('\n\n'),
		schema: notifyInputSchema,
		inputSchema: toInputSchema(notifyInputSchema, {
			allOf: [STEPS_CONDITION, NO_STEPS_UNLESS_ACTION],
		}),
	},
	{
		name: 'ask',
		description: [
			'Ask the user something in Gentbox and wait for the answer: an approval, a choice between options, or free text. It returns the answer, `deferred` (the user chose Defer: carry on with work that needs no input, then collect the answer with `wait`), or "no answer" (nothing arrived in time: the message is withdrawn, so ask in the conversation instead).',
			'Use `blocking` when you cannot continue and `deferrable` when you can do other work if the user defers. Offer `choices` for options, `allowReply` for free text; with neither the user sees Approve and Deny.',
			TITLE_RULE,
			STEPS_RULE,
			GROUPING_RULE,
			"The result repeats the hub's warnings verbatim; heed them.",
		].join('\n\n'),
		schema: askInputSchema,
		inputSchema: toInputSchema(askInputSchema, {
			allOf: [STEPS_CONDITION, NO_STEPS_UNLESS_ACTION],
		}),
	},
	{
		name: 'wait',
		description:
			'Wait for the answer to a message you sent earlier, typically after `ask` returned `deferred` and you finished the work that needed no input. Returns the answer, `deferred` again, a closed status (dismissed, withdrawn, expired), or "no answer yet": the message is not withdrawn, so you can call `wait` again or ask in the conversation.',
		schema: waitInputSchema,
		inputSchema: toInputSchema(waitInputSchema),
	},
	{
		name: 'rescind',
		description:
			'Withdraw your own open message(s) because they no longer need the user: by `messageId` (from notify or ask), or by `correlationKeys` for every open message of this conversation with those keys. Use `reason` resolved_elsewhere when you or the user handled it in the session, obsolete when it no longer matters, superseded when a newer message replaces it.',
		schema: rescindInputSchema,
		inputSchema: toInputSchema(rescindInputSchema, {
			anyOf: [{ required: ['messageId'] }, { required: ['correlationKeys'] }],
		}),
	},
	{
		name: 'create_topic',
		description: [
			"Create a topic: a project, a bigger undertaking or a category that groups threads and messages in the user's inbox. Creating an existing key returns it unchanged.",
			'`title` is a plain-words name, never a command or path. Good: "Release 1.4". Bad: "npm publish --tag next". The hub refuses a title that looks like a command.',
		].join('\n\n'),
		schema: createTopicInputSchema,
		inputSchema: toInputSchema(createTopicInputSchema),
	},
	{
		name: 'close_topic',
		description:
			'Close a topic (and its open threads) when the undertaking is over. Open messages stay unless `rescindOpenMessages` is true. A later message to a closed topic reopens it.',
		schema: closeTopicInputSchema,
		inputSchema: toInputSchema(closeTopicInputSchema),
	},
	{
		name: 'create_thread',
		description: [
			'Create a thread inside a topic: a version, a phase or a conversation. Creating an existing key returns it unchanged. With `standsForCurrentConversation` the thread stands for this conversation and takes over its automatic thread in the topic.',
			'`title` is a plain-words name, never a command or path. Good: "Phase 3 ingest". Bad: "npm publish --tag next". The hub refuses a title that looks like a command.',
		].join('\n\n'),
		schema: createThreadInputSchema,
		inputSchema: toInputSchema(createThreadInputSchema),
	},
	{
		name: 'close_thread',
		description:
			'Close a thread when its phase or version is over. Open messages stay unless `rescindOpenMessages` is true. A later message to a closed thread reopens it.',
		schema: closeThreadInputSchema,
		inputSchema: toInputSchema(closeThreadInputSchema),
	},
];
