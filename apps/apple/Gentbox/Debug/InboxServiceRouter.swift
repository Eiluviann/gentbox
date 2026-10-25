#if DEBUG || GENTBOX_HOOKS
import GentboxCore

/// The inbox's service in a debug build: the hub, or the debug panel's
/// playground while it is switched on. One inbox model reads either, so the
/// views never change model under their feet.
actor InboxServiceRouter: InboxService {
	private let hub: GentboxHub
	private var playground: PlaygroundInbox?

	init(hub: GentboxHub) {
		self.hub = hub
	}

	/// The playground from now on, or the hub again (`nil`).
	func route(to playground: PlaygroundInbox?) {
		self.playground = playground
	}

	private var service: any InboxService {
		playground ?? hub
	}

	func inbox() async throws -> Inbox {
		try await service.inbox()
	}

	func takeAction(messageId: String, actionId: String, value: String?) async throws {
		try await service.takeAction(messageId: messageId, actionId: actionId, value: value)
	}

	func snooze(messageId: String, minutes: UInt32?) async throws {
		try await service.snooze(messageId: messageId, minutes: minutes)
	}

	func dismiss(messageId: String) async throws {
		try await service.dismiss(messageId: messageId)
	}

	func muteTopic(topicId: String, isMuted: Bool) async throws {
		try await service.muteTopic(topicId: topicId, isMuted: isMuted)
	}

	func muteThread(threadId: String, isMuted: Bool) async throws {
		try await service.muteThread(threadId: threadId, isMuted: isMuted)
	}

	func markTopicRead(topicId: String) async throws {
		try await service.markTopicRead(topicId: topicId)
	}

	func markThreadRead(threadId: String) async throws {
		try await service.markThreadRead(threadId: threadId)
	}

	func closeTopic(topicId: String) async throws {
		try await service.closeTopic(topicId: topicId)
	}

	func reopenTopic(topicId: String) async throws {
		try await service.reopenTopic(topicId: topicId)
	}

	func closeThread(threadId: String) async throws {
		try await service.closeThread(threadId: threadId)
	}

	func reopenThread(threadId: String) async throws {
		try await service.reopenThread(threadId: threadId)
	}

	func moveThread(threadId: String, topicId: String) async throws {
		try await service.moveThread(threadId: threadId, topicId: topicId)
	}
}
#endif
