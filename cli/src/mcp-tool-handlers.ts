import type { CallToolResult } from '@modelcontextprotocol/sdk/types.js';
import type { z } from 'zod';
import { type AskOutcome, askAndWait, type WaitOptions } from './ask-flow.ts';
import {
	buildAnswerActions,
	buildMessage,
	describeIssues,
	type MessageDraft,
	type SenderContext,
	toTopicReference,
} from './build-message.ts';
import { resolveThreadId, resolveTopicId } from './commands/group-options.ts';
import {
	describeHubFailure,
	type HubClient,
	type HubFailure,
} from './hub-client.ts';
import {
	type AskInput,
	askInputSchema,
	type CloseThreadInput,
	type CloseTopicInput,
	type CreateThreadInput,
	type CreateTopicInput,
	closeThreadInputSchema,
	closeTopicInputSchema,
	createThreadInputSchema,
	createTopicInputSchema,
	type NotifyInput,
	notifyInputSchema,
	type RescindInput,
	rescindInputSchema,
} from './mcp-tool-schemas.ts';
import type { ActionResponse, IngestWarning } from './message-types.ts';
import type { Result } from './result.ts';

export type ToolDeps = {
	/** Connects to the hub on first use; a failure is retried on the next call. */
	getClient: () => Promise<Result<HubClient, string>>;
	sender: SenderContext;
	/** Clock and sleep for the wait loop; tests inject fakes. */
	wait?: Pick<WaitOptions, 'now' | 'sleep'>;
	defaultTimeoutSeconds: number;
};

/** Runs one tool call. Never throws: every failure is an `isError` result the model can read. */
export async function runTool(
	name: string,
	input: unknown,
	deps: ToolDeps,
	signal: AbortSignal | undefined,
): Promise<CallToolResult> {
	try {
		return await dispatch(name, input, deps, signal);
	} catch (error) {
		return errorResult(
			`unexpected failure: ${error instanceof Error ? error.message : String(error)}`,
		);
	}
}

function dispatch(
	name: string,
	input: unknown,
	deps: ToolDeps,
	signal: AbortSignal | undefined,
): Promise<CallToolResult> {
	switch (name) {
		case 'notify':
			return withInput(notifyInputSchema, input, deps, (arguments_, client) =>
				notify(arguments_, client, deps),
			);
		case 'ask':
			return withInput(askInputSchema, input, deps, (arguments_, client) =>
				ask(arguments_, client, deps, signal),
			);
		case 'rescind':
			return withInput(rescindInputSchema, input, deps, (arguments_, client) =>
				rescind(arguments_, client),
			);
		case 'create_topic':
			return withInput(
				createTopicInputSchema,
				input,
				deps,
				(arguments_, client) => createTopic(arguments_, client, deps),
			);
		case 'close_topic':
			return withInput(
				closeTopicInputSchema,
				input,
				deps,
				(arguments_, client) => closeTopic(arguments_, client),
			);
		case 'create_thread':
			return withInput(
				createThreadInputSchema,
				input,
				deps,
				(arguments_, client) => createThread(arguments_, client, deps),
			);
		case 'close_thread':
			return withInput(
				closeThreadInputSchema,
				input,
				deps,
				(arguments_, client) => closeThread(arguments_, client),
			);
		default:
			return Promise.resolve(errorResult(`unknown tool "${name}"`));
	}
}

/** Validates the arguments first (a bad call never needs the hub), then connects. */
async function withInput<T>(
	schema: z.ZodType<T>,
	input: unknown,
	deps: ToolDeps,
	run: (arguments_: T, client: HubClient) => Promise<CallToolResult>,
): Promise<CallToolResult> {
	const parsed = schema.safeParse(input ?? {});
	if (!parsed.success) {
		const lines = describeIssues(parsed.error.issues).map(
			(line) => `- ${line}`,
		);
		return errorResult(`invalid arguments:\n${lines.join('\n')}`);
	}
	const connected = await deps.getClient();
	if (!connected.ok) return errorResult(connected.error);
	return run(parsed.data, connected.value);
}

async function notify(
	input: NotifyInput,
	client: HubClient,
	deps: ToolDeps,
): Promise<CallToolResult> {
	const built = buildMessage(draftFrom(input, 'mcp.notify'), deps.sender);
	if (!built.ok) return errorResult(built.error.join('\n'));
	const posted = await client.postMessage(built.value);
	if (!posted.ok) return failureResult(posted.error);
	const { id, type, urgent, blocking, state, topicId, threadId, warnings } =
		posted.value.record;
	return jsonResult(
		{ messageId: id, type, urgent, blocking, state, topicId, threadId },
		warnings,
	);
}

async function ask(
	input: AskInput,
	client: HubClient,
	deps: ToolDeps,
	signal: AbortSignal | undefined,
): Promise<CallToolResult> {
	const type = input.type ?? 'decision_required';
	const built = buildMessage(
		{
			...draftFrom(input, 'mcp.ask'),
			type,
			blocking: input.blocking ?? 'blocking',
			actions: buildAnswerActions(type, {
				choices: input.choices ?? [],
				allowReply: input.allowReply === true,
			}),
		},
		deps.sender,
	);
	if (!built.ok) return errorResult(built.error.join('\n'));
	const asked = await askAndWait(client, built.value, {
		timeoutSeconds: input.timeoutSeconds ?? deps.defaultTimeoutSeconds,
		signal,
		...deps.wait,
	});
	if (!asked.ok) return failureResult(asked.error);
	const { ingest, outcome, didRescind } = asked.value;
	return describeAskOutcome(ingest.id, outcome, didRescind, ingest.warnings);
}

function describeAskOutcome(
	messageId: string,
	outcome: AskOutcome,
	didRescind: boolean,
	warnings: IngestWarning[] | undefined,
): CallToolResult {
	switch (outcome.kind) {
		case 'answered': {
			const { actionId, actionKind, value, answeredVia } = outcome.response;
			return jsonResult(
				{
					status: 'answered',
					messageId,
					actionId,
					actionKind,
					value,
					answeredVia,
				},
				warnings,
			);
		}
		case 'deferred':
			return jsonResult(
				{
					status: 'deferred',
					messageId,
					instruction:
						'The user chose Defer: carry on with the work that needs no input. The question stays open in their inbox; ask again later (same correlationKey replaces it) if you still need the answer.',
				},
				warnings,
			);
		case 'closed':
			return jsonResult(closedStatus(messageId, outcome.response), warnings);
		case 'timed_out':
			return jsonResult(
				{
					status: 'no_answer',
					messageId,
					reason: 'timed_out',
					messageWithdrawn: didRescind,
					instruction:
						'No answer arrived in time and the message was withdrawn. Ask the user in the conversation instead.',
				},
				warnings,
			);
		case 'not_waiting':
			return jsonResult(
				{
					status: 'no_answer',
					messageId,
					reason: 'nobody_can_answer_now',
					messageWithdrawn: didRescind,
					instruction:
						'Gentbox has no channel that can take an answer right now (the user may be in the very app you run in). Ask in the conversation instead.',
				},
				warnings,
			);
		case 'cancelled':
			return jsonResult(
				{
					status: 'no_answer',
					messageId,
					reason: 'cancelled',
					messageWithdrawn: didRescind,
				},
				warnings,
			);
		case 'failed':
			return {
				...failureResult(outcome.failure),
				content: [
					{
						type: 'text',
						text: `${describeHubFailure(outcome.failure)}\nThe answer is unknown (message ${messageId}${didRescind ? ', withdrawn' : ''}). Ask the user in the conversation instead.`,
					},
				],
			};
	}
}

function closedStatus(
	messageId: string,
	response: ActionResponse,
): Record<string, unknown> {
	return {
		status: response.status,
		messageId,
		rescindReason: response.rescindReason,
		instruction:
			'No answer will come (dismissed, withdrawn, expired or deleted). Ask the user in the conversation instead.',
	};
}

async function rescind(
	input: RescindInput,
	client: HubClient,
): Promise<CallToolResult> {
	if (input.messageId !== undefined) {
		const rescinded = await client.rescindMessage(
			input.messageId,
			input.reason ?? 'resolved_elsewhere',
		);
		return rescinded.ok
			? jsonResult(
					{
						messageId: input.messageId,
						status: rescinded.value.status,
						rescindReason: rescinded.value.rescindReason,
					},
					undefined,
				)
			: failureResult(rescinded.error);
	}
	const rescinded = await client.rescindByCorrelationKeys({
		reason: input.reason ?? 'superseded',
		correlationKeys: input.correlationKeys ?? [],
	});
	return rescinded.ok
		? jsonResult({ rescinded: rescinded.value }, undefined)
		: failureResult(rescinded.error);
}

async function createTopic(
	input: CreateTopicInput,
	client: HubClient,
	deps: ToolDeps,
): Promise<CallToolResult> {
	const created = await client.createTopic({
		...input,
		conversation: { id: deps.sender.conversationId },
	});
	return created.ok
		? jsonResult(
				{ wasCreated: created.value.wasCreated, topic: created.value.record },
				undefined,
			)
		: failureResult(created.error);
}

async function closeTopic(
	input: CloseTopicInput,
	client: HubClient,
): Promise<CallToolResult> {
	const topicId = await resolveTopicId(client, input.topic);
	if (!topicId.ok) return errorResult(topicId.error);
	const closed = await client.closeTopic(topicId.value, {
		rescindOpenMessages: input.rescindOpenMessages,
		reason: input.reason,
	});
	return closed.ok
		? jsonResult({ topic: closed.value }, undefined)
		: failureResult(closed.error);
}

async function createThread(
	input: CreateThreadInput,
	client: HubClient,
	deps: ToolDeps,
): Promise<CallToolResult> {
	const kind =
		input.standsForCurrentConversation === true
			? (input.kind ?? 'conversation')
			: input.kind;
	const created = await client.createThread({
		topic: toTopicReference(input.topic, input.topicTitle),
		key: input.key,
		title: input.title,
		summary: input.summary,
		kind,
		conversation: { id: deps.sender.conversationId },
	});
	return created.ok
		? jsonResult(
				{ wasCreated: created.value.wasCreated, thread: created.value.record },
				undefined,
			)
		: failureResult(created.error);
}

async function closeThread(
	input: CloseThreadInput,
	client: HubClient,
): Promise<CallToolResult> {
	const threadId = await resolveThreadId(client, input.thread, input.topic);
	if (!threadId.ok) return errorResult(threadId.error);
	const closed = await client.closeThread(threadId.value, {
		rescindOpenMessages: input.rescindOpenMessages,
		reason: input.reason,
	});
	return closed.ok
		? jsonResult({ thread: closed.value }, undefined)
		: failureResult(closed.error);
}

function draftFrom(input: NotifyInput | AskInput, event: string): MessageDraft {
	return {
		title: input.title,
		summary: input.summary,
		body: input.body,
		type: input.type,
		urgent: input.urgent,
		steps: input.steps,
		correlationKey: input.correlationKey,
		topic: input.topic,
		topicTitle: input.topicTitle,
		thread: input.thread,
		threadTitle: input.threadTitle,
		event,
		...('blocking' in input ? { blocking: input.blocking } : {}),
		...('scope' in input ? { scope: input.scope } : {}),
	};
}

/** The hub's warnings, word for word, as their own block after the result. */
export function formatWarnings(
	warnings: IngestWarning[] | undefined,
): string | undefined {
	if (warnings === undefined || warnings.length === 0) return undefined;
	const lines = warnings.map(
		(warning) =>
			`- [${warning.code}]${warning.field === undefined ? '' : ` ${warning.field}`}: ${warning.message}`,
	);
	return `Gentbox warnings (what the hub changed; fix this next time):\n${lines.join('\n')}`;
}

function jsonResult(
	value: Record<string, unknown>,
	warnings: IngestWarning[] | undefined,
): CallToolResult {
	const warningText = formatWarnings(warnings);
	return {
		content: [
			{ type: 'text', text: JSON.stringify(value, null, 2) },
			...(warningText === undefined
				? []
				: [{ type: 'text' as const, text: warningText }]),
		],
	};
}

function errorResult(message: string): CallToolResult {
	return { isError: true, content: [{ type: 'text', text: message }] };
}

function failureResult(failure: HubFailure): CallToolResult {
	return errorResult(describeHubFailure(failure));
}
