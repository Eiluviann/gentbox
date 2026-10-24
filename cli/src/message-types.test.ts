import { readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import type { z } from 'zod';
import {
	actionResponseSchema,
	closeRequestSchema,
	ingestResponseSchema,
	ingestWarningSchema,
	KNOWN_WARNING_CODES,
	messageSchema,
	rescindRequestSchema,
	threadCreateSchema,
	threadRecordSchema,
	topicCreateSchema,
	topicRecordSchema,
} from './message-types.ts';
import {
	createSchemaValidator,
	readJson,
	SCHEMA_DIRECTORY,
} from './test-support/schema-validator.ts';

type ExampleKind = {
	/** The JSON Schema definition the example must satisfy (`undefined` for the root, a message). */
	definition: string | undefined;
	schema: z.ZodType;
};

const MESSAGE_KIND: ExampleKind = {
	definition: undefined,
	schema: messageSchema,
};

/** Every file in docs/schema/examples must be listed: an unlisted example fails the test. */
const EXAMPLE_KINDS: Record<string, ExampleKind> = {
	'action-deferrable': MESSAGE_KIND,
	'decision-blocking': MESSAGE_KIND,
	'decision-nonblocking': MESSAGE_KIND,
	generic: MESSAGE_KIND,
	'message-inline-topic': MESSAGE_KIND,
	'message-thread-session-a': MESSAGE_KIND,
	'message-thread-session-b': MESSAGE_KIND,
	'task-completed': MESSAGE_KIND,
	'undeclared-idle': MESSAGE_KIND,
	'answered-response': {
		definition: 'actionResponse',
		schema: actionResponseSchema,
	},
	'deferred-response': {
		definition: 'actionResponse',
		schema: actionResponseSchema,
	},
	'gone-response': {
		definition: 'actionResponse',
		schema: actionResponseSchema,
	},
	'rescinded-response': {
		definition: 'actionResponse',
		schema: actionResponseSchema,
	},
	'handled-in-session-response': {
		definition: 'actionResponse',
		schema: actionResponseSchema,
	},
	'ingest-response': {
		definition: 'ingestResponse',
		schema: ingestResponseSchema,
	},
	'close-request': { definition: 'closeRequest', schema: closeRequestSchema },
	'rescind-request': {
		definition: 'rescindRequest',
		schema: rescindRequestSchema,
	},
	'topic-create': { definition: 'topicCreate', schema: topicCreateSchema },
	'topic-record': { definition: 'topicRecord', schema: topicRecordSchema },
	'topic-record-general': {
		definition: 'topicRecord',
		schema: topicRecordSchema,
	},
	'thread-conversation-takeover': {
		definition: 'threadCreate',
		schema: threadCreateSchema,
	},
	'thread-record': { definition: 'threadRecord', schema: threadRecordSchema },
	'thread-record-implicit': {
		definition: 'threadRecord',
		schema: threadRecordSchema,
	},
};

const exampleNames = readdirSync(join(SCHEMA_DIRECTORY, 'examples'))
	.filter((fileName) => fileName.endsWith('.json'))
	.map((fileName) => fileName.replace(/\.json$/, ''));

describe('docs/schema/examples', () => {
	it('lists every example file', () => {
		expect(exampleNames.sort()).toEqual(Object.keys(EXAMPLE_KINDS).sort());
	});

	describe.each(exampleNames)('%s', (name) => {
		const kind = EXAMPLE_KINDS[name];
		const example = readJson(
			join(SCHEMA_DIRECTORY, 'examples', `${name}.json`),
		);

		it('is valid JSON Schema', () => {
			const validate = createSchemaValidator(kind?.definition);
			expect(validate(example), JSON.stringify(validate.errors)).toBe(true);
		});

		it('parses into the TypeScript types without losing a field', () => {
			const parsed = kind?.schema.safeParse(example);
			expect(parsed?.success, JSON.stringify(parsed?.error?.issues)).toBe(true);
			expect(parsed?.data).toEqual(example);
		});
	});
});

describe('the schema rules that JSON Schema keeps in allOf', () => {
	const base = {
		version: 1,
		source: { agent: 'claude-code' },
		conversation: { id: 'c1' },
		title: 'Something happened',
	};

	it.each([
		[
			'a blocking task_completed',
			{ type: 'task_completed', blocking: 'blocking' },
		],
		['an urgent generic', { type: 'generic', urgent: true }],
		['a scope on a decision', { type: 'decision_required', scope: 'goal' }],
		[
			'steps on a decision',
			{ type: 'decision_required', steps: [{ text: 'Do it' }] },
		],
		['an unknown top-level field', { colour: 'red' }],
	])('both reject %s', (_name, extra) => {
		const message = { ...base, ...extra };
		const validate = createSchemaValidator(undefined);
		expect(validate(message)).toBe(false);
		expect(messageSchema.safeParse(message).success).toBe(false);
	});

	it('both accept a message with only the required fields', () => {
		expect(createSchemaValidator(undefined)(base)).toBe(true);
		expect(messageSchema.safeParse(base).success).toBe(true);
	});
});

describe('the reasons a sender names and the one the hub records', () => {
	it('lets a response carry handled_in_session, in the schema and in the types', () => {
		const response = {
			messageId: '01JBM3Z8Q7Y4K2N6P0R5T9VWXA',
			status: 'rescinded',
			rescindReason: 'handled_in_session',
		};
		expect(createSchemaValidator('actionResponse')(response)).toBe(true);
		expect(actionResponseSchema.safeParse(response).success).toBe(true);
	});

	it('reads a rescind reason it has never seen, so a newer hub never breaks a poll', () => {
		const parsed = actionResponseSchema.safeParse({
			messageId: '01JBM3Z8Q7Y4K2N6P0R5T9VWXA',
			status: 'rescinded',
			rescindReason: 'from_the_future',
		});
		expect(parsed).toMatchObject({
			success: true,
			data: { rescindReason: 'from_the_future' },
		});
	});

	it.each([
		['rescindRequest', rescindRequestSchema, { reason: 'handled_in_session' }],
		[
			'closeRequest',
			closeRequestSchema,
			{ rescindOpenMessages: true, reason: 'handled_in_session' },
		],
	])(
		'refuses handled_in_session in a %s: only the hub records it',
		(definition, schema, request) => {
			expect(createSchemaValidator(definition)(request)).toBe(false);
			expect(schema.safeParse(request).success).toBe(false);
		},
	);

	it.each(['resolved_elsewhere', 'obsolete', 'superseded'])(
		'still accepts %s in a rescind request',
		(reason) => {
			expect(createSchemaValidator('rescindRequest')({ reason })).toBe(true);
			expect(rescindRequestSchema.safeParse({ reason }).success).toBe(true);
		},
	);
});

describe('what is read from the hub is lenient, what is sent is strict', () => {
	it('lists every warning code the JSON Schema knows', () => {
		const schema = readJson(join(SCHEMA_DIRECTORY, 'message-v1.json'));
		const codes =
			JSON.stringify(schema).match(/"title_looks_like_command"[^\]]*\]/)?.[0] ??
			'';
		for (const code of KNOWN_WARNING_CODES)
			expect(codes).toContain(`"${code}"`);
		expect(codes.match(/"[a-z_]+"/g)).toHaveLength(KNOWN_WARNING_CODES.length);
	});

	it('accepts a warning code and a field it has never seen', () => {
		const parsed = ingestWarningSchema.safeParse({
			code: 'from_the_future',
			message: 'm',
			extra: 1,
		});
		expect(parsed).toMatchObject({
			success: true,
			data: { code: 'from_the_future' },
		});
	});

	it('still rejects an unknown field in a message the client would send', () => {
		expect(
			topicCreateSchema.safeParse({ key: 'k', title: 't', colour: 'red' })
				.success,
		).toBe(false);
	});
});
