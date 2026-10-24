import { z } from 'zod';
import { toTopicReference } from '../build-message.ts';
import { isUlid } from '../message-types.ts';
import {
	type CommandContext,
	closeError,
	EXIT,
	type ExitCode,
	hubError,
	parseArguments,
	printJson,
	usageError,
	withHub,
} from './command-context.ts';
import {
	CLOSE_OPTIONS,
	CLOSE_OPTIONS_HELP,
	readCloseRequest,
	resolveTopicId,
} from './group-options.ts';

export const THREAD_HELP = `gentbox thread create <key> --title <text> [--topic <key|id>] [--topic-title <text>] [--summary <text>] [--kind <kind>] [--this-conversation]
gentbox thread list [--topic <key|id>] [--status open|closed]
gentbox thread close <id> [--rescind-open] [--reason <reason>]
gentbox thread reopen <id>
gentbox thread move <id> --topic <key|id> [--topic-title <text>]

A thread is a version, a phase or a conversation inside a topic. The key is yours and stable within
its topic; creating an existing key returns it unchanged. The title is a plain-words name.
--this-conversation (with kind "conversation") makes the thread stand for this conversation, taking
over its automatic thread in the topic.

${CLOSE_OPTIONS_HELP}`;

const THREAD_OPTIONS = {
	...CLOSE_OPTIONS,
	title: { type: 'string' },
	summary: { type: 'string' },
	kind: { type: 'string' },
	topic: { type: 'string' },
	'topic-title': { type: 'string' },
	'this-conversation': { type: 'boolean' },
	status: { type: 'string' },
} as const;

const statusSchema = z.enum(['open', 'closed']);

type ThreadValues = {
	title?: string | undefined;
	summary?: string | undefined;
	kind?: string | undefined;
	topic?: string | undefined;
	'topic-title'?: string | undefined;
	'this-conversation'?: boolean | undefined;
	status?: string | undefined;
	'rescind-open'?: boolean | undefined;
	reason?: string | undefined;
};

export async function runThread(
	args: string[],
	context: CommandContext,
): Promise<ExitCode> {
	const parsed = parseArguments(args, THREAD_OPTIONS);
	if (!parsed.ok) return usageError(context, parsed.error);
	const { values, positionals } = parsed.value;
	const [action, reference, ...rest] = positionals;
	if (rest.length > 0) return usageError(context, 'too many arguments');
	switch (action) {
		case 'create':
			return createThread(reference, values, context);
		case 'list':
			return listThreads(values, context);
		case 'close':
			return closeThread(reference, values, context);
		case 'reopen':
			return reopenThread(reference, context);
		case 'move':
			return moveThread(reference, values, context);
		default:
			return usageError(
				context,
				'use `gentbox thread create|list|close|reopen|move` (see `gentbox help thread`)',
			);
	}
}

async function createThread(
	key: string | undefined,
	values: ThreadValues,
	context: CommandContext,
): Promise<ExitCode> {
	if (key === undefined)
		return usageError(context, 'thread create needs a key');
	if (values.title === undefined)
		return usageError(context, '--title is required');
	const kind =
		values['this-conversation'] === true
			? (values.kind ?? 'conversation')
			: values.kind;
	const request = {
		topic: toTopicReference(values.topic, values['topic-title']),
		key,
		title: values.title,
		summary: values.summary,
		kind,
		conversation: { id: context.sender.conversationId },
	};
	return withHub(context, async (client) => {
		const created = await client.createThread(request);
		if (!created.ok) return hubError(context, created.error);
		printJson(context, created.value.record);
		return EXIT.ok;
	});
}

async function listThreads(
	values: ThreadValues,
	context: CommandContext,
): Promise<ExitCode> {
	const status =
		values.status === undefined
			? undefined
			: statusSchema.safeParse(values.status);
	if (status !== undefined && !status.success)
		return usageError(context, '--status is open or closed');
	return withHub(context, async (client) => {
		const topicId =
			values.topic === undefined
				? undefined
				: await resolveTopicId(client, values.topic);
		if (topicId !== undefined && !topicId.ok)
			return usageError(context, topicId.error);
		const listed = await client.listThreads(topicId?.value, status?.data);
		if (!listed.ok) return hubError(context, listed.error);
		printJson(context, { threads: listed.value });
		return EXIT.ok;
	});
}

async function closeThread(
	threadId: string | undefined,
	values: ThreadValues,
	context: CommandContext,
): Promise<ExitCode> {
	if (threadId === undefined || !isUlid(threadId))
		return usageError(context, 'thread close needs a thread id');
	const request = readCloseRequest(values);
	if (!request.ok) return usageError(context, request.error);
	return withHub(context, async (client) => {
		const closed = await client.closeThread(threadId, request.value);
		if (!closed.ok) return closeError(context, closed.error, 'thread');
		printJson(context, closed.value);
		return EXIT.ok;
	});
}

async function reopenThread(
	threadId: string | undefined,
	context: CommandContext,
): Promise<ExitCode> {
	if (threadId === undefined || !isUlid(threadId))
		return usageError(context, 'thread reopen needs a thread id');
	return withHub(context, async (client) => {
		const reopened = await client.reopenThread(threadId);
		if (!reopened.ok) return hubError(context, reopened.error);
		printJson(context, reopened.value);
		return EXIT.ok;
	});
}

async function moveThread(
	threadId: string | undefined,
	values: ThreadValues,
	context: CommandContext,
): Promise<ExitCode> {
	const topic = toTopicReference(values.topic, values['topic-title']);
	if (threadId === undefined || !isUlid(threadId) || topic === undefined)
		return usageError(context, 'thread move needs a thread id and --topic');
	return withHub(context, async (client) => {
		const moved = await client.moveThread(threadId, { topic });
		if (!moved.ok) return hubError(context, moved.error);
		printJson(context, moved.value);
		return EXIT.ok;
	});
}
