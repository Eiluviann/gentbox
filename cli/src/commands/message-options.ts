import { z } from 'zod';
import type { MessageDraft } from '../build-message.ts';
import {
	type ActionStep,
	actionStepSchema,
	blockingLevelSchema,
	messageTypeSchema,
	type RescindReason,
	rescindReasonSchema,
} from '../message-types.ts';
import { fail, type Result, succeed } from '../result.ts';
import type { CommandContext } from './command-context.ts';

/** Flags shared by `send` and `ask`. */
export const MESSAGE_OPTIONS = {
	title: { type: 'string' },
	summary: { type: 'string' },
	body: { type: 'string' },
	'body-file': { type: 'string' },
	type: { type: 'string' },
	urgent: { type: 'boolean' },
	blocking: { type: 'string' },
	scope: { type: 'string' },
	step: { type: 'string', multiple: true },
	topic: { type: 'string' },
	'topic-title': { type: 'string' },
	thread: { type: 'string' },
	'thread-title': { type: 'string' },
	event: { type: 'string' },
	'correlation-key': { type: 'string' },
	'idempotency-key': { type: 'string' },
	sensitive: { type: 'boolean' },
	'expires-at': { type: 'string' },
	'rescind-key': { type: 'string', multiple: true },
	'rescind-reason': { type: 'string' },
} as const;

export type MessageOptionValues = {
	title?: string | undefined;
	summary?: string | undefined;
	body?: string | undefined;
	'body-file'?: string | undefined;
	type?: string | undefined;
	urgent?: boolean | undefined;
	blocking?: string | undefined;
	scope?: string | undefined;
	step?: string[] | undefined;
	topic?: string | undefined;
	'topic-title'?: string | undefined;
	thread?: string | undefined;
	'thread-title'?: string | undefined;
	event?: string | undefined;
	'correlation-key'?: string | undefined;
	'idempotency-key'?: string | undefined;
	sensitive?: boolean | undefined;
	'expires-at'?: string | undefined;
	'rescind-key'?: string[] | undefined;
	'rescind-reason'?: string | undefined;
};

export const MESSAGE_OPTIONS_HELP = `  --title <text>            Required. A plain-words summary of what you ask or did, one line
                            (e.g. "Allow Bash to run the ingest tests?"). Never a command,
                            path or URL: those go in --body.
  --summary <text>          One or two plain sentences (max 400 characters).
  --body <markdown>         Commands, paths, URLs and raw input, shown as code boxes.
  --body-file <path|->      Read the body from a file, or from stdin with -.
  --type <type>             decision_required | action_required | task_completed | generic
  --urgent / --no-urgent    Break through quiet hours and Focus.
  --blocking <level>        non_blocking | deferrable | blocking
  --scope <task|goal>       For task_completed.
  --step <text[ :: detail]> One step of an action plan; repeat for more. Required for
                            --type action_required. Commands go in the detail.
  --topic <key|id>          Inline topic: a key creates it on first use. --topic-title names it.
  --thread <key|id>         Inline thread: a key creates it on first use. --thread-title names it.
  --event <name>            Your own dotted event name, for rules.
  --correlation-key <key>   Same key replaces your open message with it.
  --idempotency-key <key>   Makes retries safe.
  --sensitive               Keep even the title off other devices.
  --expires-at <iso-time>   When the message expires.
  --rescind-key <key>       Withdraw your open messages with this correlation key; repeatable.
  --rescind-reason <reason> resolved_elsewhere | obsolete | superseded (default superseded).`;

const STEP_DETAIL_SEPARATOR = ' :: ';

/** Turns the shared flags into a draft. Every problem is listed, none is guessed away. */
export async function readMessageDraft(
	values: MessageOptionValues,
	context: CommandContext,
): Promise<Result<MessageDraft, string[]>> {
	const problems: string[] = [];
	if (values.title === undefined) problems.push('--title is required');
	const type = parseChoice(
		'--type',
		messageTypeSchema,
		messageTypeSchema.options,
		values.type,
		problems,
	);
	const blocking = parseChoice(
		'--blocking',
		blockingLevelSchema,
		blockingLevelSchema.options,
		values.blocking,
		problems,
	);
	const scope = parseChoice(
		'--scope',
		z.enum(['task', 'goal']),
		['task', 'goal'],
		values.scope,
		problems,
	);
	const rescindReason = parseChoice(
		'--rescind-reason',
		rescindReasonSchema,
		rescindReasonSchema.options,
		values['rescind-reason'],
		problems,
	);
	const steps = parseSteps(values.step, problems);
	const body = await readBody(values, context, problems);
	const rescindKeys = values['rescind-key'];
	const rescinds =
		rescindKeys === undefined || rescindKeys.length === 0
			? undefined
			: {
					reason: rescindReason ?? ('superseded' satisfies RescindReason),
					correlationKeys: rescindKeys,
				};
	if (rescindKeys === undefined && rescindReason !== undefined) {
		problems.push('--rescind-reason needs at least one --rescind-key');
	}
	if (problems.length > 0 || values.title === undefined) return fail(problems);
	return succeed({
		title: values.title,
		summary: values.summary,
		body,
		type,
		urgent: values.urgent,
		blocking,
		scope,
		steps,
		topic: values.topic,
		topicTitle: values['topic-title'],
		thread: values.thread,
		threadTitle: values['thread-title'],
		event: values.event,
		correlationKey: values['correlation-key'],
		idempotencyKey: values['idempotency-key'],
		sensitive: values.sensitive,
		expiresAt: values['expires-at'],
		rescinds,
	});
}

function parseChoice<T extends string>(
	flag: string,
	schema: z.ZodType<T>,
	allowed: readonly string[],
	value: string | undefined,
	problems: string[],
): T | undefined {
	if (value === undefined) return undefined;
	const parsed = schema.safeParse(value);
	if (parsed.success) return parsed.data;
	problems.push(
		`${flag} must be one of: ${allowed.join(', ')} (got "${value}")`,
	);
	return undefined;
}

/** `--step "Sign in to npm :: ```npm login```"`: the text, then an optional detail. */
export function parseSteps(
	rawSteps: string[] | undefined,
	problems: string[],
): ActionStep[] | undefined {
	if (rawSteps === undefined || rawSteps.length === 0) return undefined;
	const steps: ActionStep[] = [];
	for (const raw of rawSteps) {
		const separatorAt = raw.indexOf(STEP_DETAIL_SEPARATOR);
		const text = (separatorAt === -1 ? raw : raw.slice(0, separatorAt)).trim();
		const detail =
			separatorAt === -1
				? undefined
				: raw.slice(separatorAt + STEP_DETAIL_SEPARATOR.length).trim();
		const step = actionStepSchema.safeParse({
			text,
			detail: detail === '' ? undefined : detail,
		});
		if (step.success) steps.push(step.data);
		else
			problems.push(
				`--step "${raw.slice(0, 40)}": ${step.error.issues[0]?.message ?? 'invalid'}`,
			);
	}
	return steps;
}

async function readBody(
	values: MessageOptionValues,
	context: CommandContext,
	problems: string[],
): Promise<string | undefined> {
	const file = values['body-file'];
	if (file === undefined) return values.body;
	if (values.body !== undefined)
		problems.push('use --body or --body-file, not both');
	try {
		return file === '-' ? await context.readStdin() : context.readFile(file);
	} catch (error) {
		problems.push(
			`cannot read ${file}: ${error instanceof Error ? error.message : String(error)}`,
		);
		return undefined;
	}
}
