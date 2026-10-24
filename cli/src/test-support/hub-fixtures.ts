import type {
	ActionResponse,
	IngestResponse,
	MessageV1,
	ResponseStatus,
} from '../message-types.ts';

export const MESSAGE_ID = '01JBMA2W8R6T4Y2V0P8N6M4K2H';
export const TOPIC_ID = '01JBM6R3K9P2W5X8Z1C4V7N0QD';
export const THREAD_ID = '01JBM9P3H4S5E6J7N8G9E0S1TQ';

export function ingestResponse(
	overrides: Partial<IngestResponse> = {},
): IngestResponse {
	return {
		id: MESSAGE_ID,
		type: 'decision_required',
		urgent: false,
		blocking: 'blocking',
		topicId: TOPIC_ID,
		threadId: THREAD_ID,
		state: 'open',
		shouldWait: true,
		warnings: [],
		...overrides,
	};
}

export function actionResponse(
	status: ResponseStatus,
	overrides: Partial<ActionResponse> = {},
): ActionResponse {
	return { messageId: MESSAGE_ID, status, ...overrides };
}

export const SAMPLE_MESSAGE: MessageV1 = {
	version: 1,
	source: { agent: 'claude-code' },
	conversation: { id: 'session-1' },
	title: 'Allow Bash to run the ingest tests?',
	type: 'decision_required',
	blocking: 'blocking',
};
