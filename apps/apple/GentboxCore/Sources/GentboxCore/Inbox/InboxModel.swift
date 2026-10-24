import Foundation
import Observation

/// What the inbox needs from the hub. `GentboxHub` conforms as generated;
/// tests pass a fake.
public protocol InboxService: AnyObject, Sendable {
	func inbox() async throws -> Inbox
	func takeAction(messageId: String, actionId: String, value: String?) async throws
	func snooze(messageId: String, minutes: UInt32?) async throws
	func dismiss(messageId: String) async throws
	func muteTopic(topicId: String, isMuted: Bool) async throws
	func muteThread(threadId: String, isMuted: Bool) async throws
	func markTopicRead(topicId: String) async throws
	func markThreadRead(threadId: String) async throws
	func closeTopic(topicId: String, dismissOpenMessages: Bool) async throws
	func reopenTopic(topicId: String) async throws
	func closeThread(threadId: String, dismissOpenMessages: Bool) async throws
	func reopenThread(threadId: String) async throws
	func moveThread(threadId: String, topicId: String) async throws
}

extension GentboxHub: InboxService {}

/// The menu-bar inbox's state on the main actor: the grouped inbox, read
/// again whenever the hub reports a change, and the commands you send from
/// it. Reads coalesce: a burst of events costs one or two reads, never one
/// per event.
@MainActor
@Observable
public final class InboxModel {
	/// The inbox as last read; `nil` until the first read finishes.
	public private(set) var inbox: Inbox?
	/// What went wrong, in words, for the popover's foot: the last command
	/// that failed ("Already answered"), else the last read that failed.
	public var failure: String? {
		commandFailure ?? readFailure
	}

	/// A command's failure stays until the next command works: the read that
	/// follows every command must not wipe it.
	private var commandFailure: String?
	/// A read's failure clears with the next read that works.
	private var readFailure: String?
	/// The row you picked; it takes the full card until it leaves the list.
	public var selectedMessageId: String?

	@ObservationIgnored private let service: any InboxService
	@ObservationIgnored private var isReading = false
	@ObservationIgnored private var isReadAgainNeeded = false

	public init(service: any InboxService) {
		self.service = service
	}

	/// The messages agents wait on: the menu-bar count.
	public var waitingCount: Int {
		Int(inbox?.waitingCount ?? 0)
	}

	/// Whether an open message is still unread: the menu-bar dot.
	public var hasUnread: Bool {
		inbox?.hasUnread ?? false
	}

	/// Whether nothing is listed at all.
	public var isEmpty: Bool {
		inbox?.items.isEmpty ?? true
	}

	/// The message in the full card (design: "Next up"): the row you picked,
	/// else the one an agent is blocked on, else none.
	public var card: InboxMessage? {
		guard let inbox else { return nil }
		if let selectedMessageId, let picked = inbox.items.first(where: { $0.id == selectedMessageId }) {
			return picked
		}
		return inbox.nextUp
	}

	/// The two-line list: everything but the card, most pressing first.
	public var list: [InboxMessage] {
		let cardId = card?.id
		return (inbox?.items ?? []).filter { $0.id != cardId }
	}

	/// Any hub event: read the inbox again.
	public func handle(event _: HubEvent) {
		requestRead()
	}

	/// Reads the inbox now, or once more after the read in flight.
	public func requestRead() {
		if isReading {
			isReadAgainNeeded = true
			return
		}
		isReading = true
		Task { await readUntilCurrent() }
	}

	/// Reads until no event arrived during the last read; returns when the
	/// inbox is current. Tests await it directly.
	public func readUntilCurrent() async {
		isReading = true
		repeat {
			isReadAgainNeeded = false
			do {
				inbox = try await service.inbox()
				readFailure = nil
			} catch {
				readFailure = Self.words(for: error)
			}
		} while isReadAgainNeeded
		isReading = false
	}

	// MARK: Commands

	/// A button on a message.
	public func press(_ action: InboxAction, on message: InboxMessage, reply: String? = nil) async {
		await run { try await $0.takeAction(messageId: message.id, actionId: action.id, value: reply) }
	}

	/// Remind me later; never reaches the agent.
	public func snooze(_ message: InboxMessage, minutes: UInt32 = 15) async {
		await run { try await $0.snooze(messageId: message.id, minutes: minutes) }
	}

	/// Clears a message without answering it.
	public func dismiss(_ message: InboxMessage) async {
		await run { try await $0.dismiss(messageId: message.id) }
	}

	public func setTopicMuted(_ topicId: String, _ isMuted: Bool) async {
		await run { try await $0.muteTopic(topicId: topicId, isMuted: isMuted) }
	}

	public func setThreadMuted(_ threadId: String, _ isMuted: Bool) async {
		await run { try await $0.muteThread(threadId: threadId, isMuted: isMuted) }
	}

	public func markTopicRead(_ topicId: String) async {
		await run { try await $0.markTopicRead(topicId: topicId) }
	}

	public func markThreadRead(_ threadId: String) async {
		await run { try await $0.markThreadRead(threadId: threadId) }
	}

	/// Closes a topic; with `dismissingOpenMessages` its open messages are
	/// dismissed (never withdrawn: that is the agent's word).
	public func closeTopic(_ topicId: String, dismissingOpenMessages: Bool) async {
		await run { try await $0.closeTopic(topicId: topicId, dismissOpenMessages: dismissingOpenMessages) }
	}

	/// Closes a thread; see `closeTopic`.
	public func closeThread(_ threadId: String, dismissingOpenMessages: Bool) async {
		await run { try await $0.closeThread(threadId: threadId, dismissOpenMessages: dismissingOpenMessages) }
	}

	public func reopen(_ group: ClosedGroup) async {
		switch group.kind {
		case .topic: await run { try await $0.reopenTopic(topicId: group.id) }
		case .thread: await run { try await $0.reopenThread(threadId: group.id) }
		}
	}

	public func moveThread(_ threadId: String, toTopic topicId: String) async {
		await run { try await $0.moveThread(threadId: threadId, topicId: topicId) }
	}

	/// Runs a command, then reads the inbox: the hub's events would too, but
	/// the read makes the result visible even when no event follows.
	private func run(_ command: @Sendable (any InboxService) async throws -> Void) async {
		do {
			try await command(service)
			commandFailure = nil
		} catch {
			commandFailure = Self.words(for: error)
		}
		requestRead()
	}

	/// A failure in words for the popover's foot.
	public static func words(for error: any Error) -> String {
		switch error as? GentboxError {
		case .AlreadyClosed(let state):
			switch state {
			case .answered: return "Already answered"
			case .rescinded: return "Withdrawn by the agent"
			case .expired: return "Expired before your answer"
			default: return "Already closed"
			}
		case .NotFound: return "No longer there"
		case .Refused(let message): return message
		case .Stopped: return "Gentbox has stopped"
		case .AnotherHubRunning(let message), .CannotStart(let message), .Internal(let message):
			return message
		case nil: return error.localizedDescription
		}
	}
}
