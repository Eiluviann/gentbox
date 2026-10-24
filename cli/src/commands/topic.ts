import { z } from 'zod';
import { topicStyleSchema } from '../message-types.ts';
import {
	type CommandContext,
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

export const TOPIC_HELP = `gentbox topic create <key> --title <text> [--summary <text>] [--kind <kind>] [--symbol <sf-symbol>] [--color <color>]
gentbox topic list [--status open|closed]
gentbox topic close <id|key> [--rescind-open] [--reason <reason>]
gentbox topic reopen <id|key>

A topic is a project, a bigger undertaking or a category. The key is yours and stable
(e.g. release-1.4, repo:gentbox); creating an existing key returns it unchanged. The title is a
plain-words name, never a command or path.

${CLOSE_OPTIONS_HELP}`;

const TOPIC_OPTIONS = {
	...CLOSE_OPTIONS,
	title: { type: 'string' },
	summary: { type: 'string' },
	kind: { type: 'string' },
	symbol: { type: 'string' },
	color: { type: 'string' },
	status: { type: 'string' },
} as const;

const statusSchema = z.enum(['open', 'closed']);

export async function runTopic(
	args: string[],
	context: CommandContext,
): Promise<ExitCode> {
	const parsed = parseArguments(args, TOPIC_OPTIONS);
	if (!parsed.ok) return usageError(context, parsed.error);
	const { values, positionals } = parsed.value;
	const [action, reference, ...rest] = positionals;
	if (rest.length > 0) return usageError(context, 'too many arguments');
	switch (action) {
		case 'create':
			return createTopic(reference, values, context);
		case 'list': {
			const status =
				values.status === undefined
					? undefined
					: statusSchema.safeParse(values.status);
			if (status !== undefined && !status.success)
				return usageError(context, '--status is open or closed');
			return withHub(context, async (client) => {
				const listed = await client.listTopics(status?.data);
				if (!listed.ok) return hubError(context, listed.error);
				printJson(context, { topics: listed.value });
				return EXIT.ok;
			});
		}
		case 'close': {
			if (reference === undefined)
				return usageError(context, 'topic close needs a topic id or key');
			const request = readCloseRequest(values);
			if (!request.ok) return usageError(context, request.error);
			return withHub(context, async (client) => {
				const topicId = await resolveTopicId(client, reference);
				if (!topicId.ok) return usageError(context, topicId.error);
				const closed = await client.closeTopic(topicId.value, request.value);
				if (!closed.ok) return hubError(context, closed.error);
				printJson(context, closed.value);
				return EXIT.ok;
			});
		}
		case 'reopen': {
			if (reference === undefined)
				return usageError(context, 'topic reopen needs a topic id or key');
			return withHub(context, async (client) => {
				const topicId = await resolveTopicId(client, reference);
				if (!topicId.ok) return usageError(context, topicId.error);
				const reopened = await client.reopenTopic(topicId.value);
				if (!reopened.ok) return hubError(context, reopened.error);
				printJson(context, reopened.value);
				return EXIT.ok;
			});
		}
		default:
			return usageError(
				context,
				'use `gentbox topic create|list|close|reopen` (see `gentbox help topic`)',
			);
	}
}

async function createTopic(
	key: string | undefined,
	values: {
		title?: string | undefined;
		summary?: string | undefined;
		kind?: string | undefined;
		symbol?: string | undefined;
		color?: string | undefined;
	},
	context: CommandContext,
): Promise<ExitCode> {
	if (key === undefined) return usageError(context, 'topic create needs a key');
	if (values.title === undefined)
		return usageError(context, '--title is required');
	const style = topicStyleSchema.safeParse({
		symbol: values.symbol,
		color: values.color,
	});
	if (!style.success)
		return usageError(
			context,
			`--symbol or --color is invalid: ${style.error.issues[0]?.message ?? ''}`,
		);
	const hasStyle = values.symbol !== undefined || values.color !== undefined;
	const request = {
		key,
		title: values.title,
		summary: values.summary,
		kind: values.kind,
		style: hasStyle ? style.data : undefined,
		conversation: { id: context.sender.conversationId },
	};
	return withHub(context, async (client) => {
		const created = await client.createTopic(request);
		if (!created.ok) return hubError(context, created.error);
		printJson(context, created.value.record);
		return EXIT.ok;
	});
}
