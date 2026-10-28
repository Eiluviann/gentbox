#if DEBUG || GENTBOX_HOOKS
import GentboxCore

/// The inbox's service in a debug build: the hub, or the debug panel's
/// playground while it is switched on. One inbox model reads either, so the
/// views never change model under their feet. A command goes where the
/// item it names lives, whichever shows now, so one issued just before a
/// switch never reaches the other side: the router notes every id the
/// playground's list shows, and only those go to the playground.
actor InboxServiceRouter: InboxService {
	private let hub: GentboxHub
	/// The playground once used: commands for its items reach it after a
	/// switch back to the hub.
	private var playground: PlaygroundInbox?
	private var isShowingPlayground = false
	/// The ids of every message, topic and thread the playground has shown.
	private var playgroundIds: Set<String> = []

	init(hub: GentboxHub) {
		self.hub = hub
	}

	/// The playground from now on, or the hub again (`nil`).
	func route(to playground: PlaygroundInbox?) {
		if let playground {
			self.playground = playground
		}
		isShowingPlayground = playground != nil
	}

	/// Where the item named `id` lives.
	private func service(for id: String) -> any InboxService {
		guard playgroundIds.contains(id), let playground else { return hub }
		return playground
	}

	func inbox() async throws -> Inbox {
		guard isShowingPlayground, let playground else {
			return try await hub.inbox()
		}
		let inbox = await playground.inbox()
		for message in inbox.items {
			playgroundIds.formUnion([message.id, message.topicId, message.threadId])
		}
		return inbox
	}

	func takeAction(messageId: String, actionId: String, value: String?) async throws {
		try await service(for: messageId).takeAction(messageId: messageId, actionId: actionId, value: value)
	}

	func snooze(messageId: String, minutes: UInt32?) async throws {
		try await service(for: messageId).snooze(messageId: messageId, minutes: minutes)
	}

	func dismiss(messageId: String) async throws {
		try await service(for: messageId).dismiss(messageId: messageId)
	}

	func muteTopic(topicId: String, isMuted: Bool) async throws {
		try await service(for: topicId).muteTopic(topicId: topicId, isMuted: isMuted)
	}

	func muteThread(threadId: String, isMuted: Bool) async throws {
		try await service(for: threadId).muteThread(threadId: threadId, isMuted: isMuted)
	}

	func markTopicRead(topicId: String) async throws {
		try await service(for: topicId).markTopicRead(topicId: topicId)
	}

	func markThreadRead(threadId: String) async throws {
		try await service(for: threadId).markThreadRead(threadId: threadId)
	}

	func closeTopic(topicId: String) async throws {
		try await service(for: topicId).closeTopic(topicId: topicId)
	}

	func reopenTopic(topicId: String) async throws {
		try await service(for: topicId).reopenTopic(topicId: topicId)
	}

	func closeThread(threadId: String) async throws {
		try await service(for: threadId).closeThread(threadId: threadId)
	}

	func reopenThread(threadId: String) async throws {
		try await service(for: threadId).reopenThread(threadId: threadId)
	}

	func moveThread(threadId: String, topicId: String) async throws {
		try await service(for: threadId).moveThread(threadId: threadId, topicId: topicId)
	}
}
#endif
