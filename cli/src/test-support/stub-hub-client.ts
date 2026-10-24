import type { HubClient } from '../hub-client.ts';

/** A client whose every call throws until a test overrides it: a call a test did not expect is a failure. */
export function stubHubClient(overrides: Partial<HubClient> = {}): HubClient {
	const unexpected = (name: string) => () => {
		throw new Error(`unexpected hub call: ${name}`);
	};
	return {
		postMessage: unexpected('postMessage'),
		getResponse: unexpected('getResponse'),
		rescindMessage: unexpected('rescindMessage'),
		rescindByCorrelationKeys: unexpected('rescindByCorrelationKeys'),
		reportProgress: unexpected('reportProgress'),
		setFocus: unexpected('setFocus'),
		createTopic: unexpected('createTopic'),
		listTopics: unexpected('listTopics'),
		closeTopic: unexpected('closeTopic'),
		reopenTopic: unexpected('reopenTopic'),
		createThread: unexpected('createThread'),
		listThreads: unexpected('listThreads'),
		closeThread: unexpected('closeThread'),
		reopenThread: unexpected('reopenThread'),
		moveThread: unexpected('moveThread'),
		...overrides,
	};
}
