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
	func closeTopic(topicId: String) async throws
	func reopenTopic(topicId: String) async throws
	func closeThread(threadId: String) async throws
	func reopenThread(threadId: String) async throws
	func moveThread(threadId: String, topicId: String) async throws
}

extension GentboxHub: InboxService {}

/// The menu-bar inbox's state on the main actor: the inbox, read again
/// whenever the hub reports a change; which item is expanded (an accordion:
/// one at most); a filter to one thread's open items; the rows picked for a
/// bulk Dismiss; and the commands you send. Reads coalesce: a burst of events
/// costs one or two reads, never one per event.
@MainActor
@Observable
public final class InboxModel {
	/// What is expanded: the full card, or (PR 4) the undo bar of an answer
	/// held for 3 s in place of the card (design: `mac-popover-undo`).
	public enum Expanded: Equatable, Sendable {
		case card(String)
		case held(String)

		public var messageId: String {
			switch self {
			case .card(let id), .held(let id): id
			}
		}
	}

	/// What Close did.
	public enum CloseOutcome: Equatable, Sendable {
		case closed
		/// Not closed: this many items are still open; the list now shows them.
		case stillOpen(Int)
		/// Not closed: the hub refused or failed; `failure` says why.
		case failed
	}

	/// The list narrowed to one thread's open items, after Close found some.
	/// Its count is read live (`filterOpenCount`), so it falls as items close.
	public struct GroupFilter: Equatable, Sendable {
		public let threadId: String
		public let title: String
	}

	/// How the expanded item is chosen.
	private enum ExpansionChoice: Equatable {
		/// The next-up item: a waiting sender first, then the blocking level.
		/// Only while the inbox is hidden: once shown, the default is pinned.
		case followNextUp
		/// The item you expanded, until it leaves the list.
		case chosen(Expanded)
		/// You collapsed the expanded item: nothing, until you expand one.
		case collapsed
	}

	/// The inbox as last read; `nil` until the first read finishes.
	public private(set) var inbox: Inbox?
	/// What went wrong, in words, for the popover's foot: the last command
	/// that failed ("Already answered"), else the last read that failed.
	public var failure: String? {
		commandFailure ?? readFailure
	}
	/// The list narrowed to one thread, or `nil` for everything.
	public private(set) var filter: GroupFilter?
	/// The rows picked for a bulk Dismiss.
	public private(set) var selection: Set<String> = []

	/// A command's failure stays until the next command works: the read that
	/// follows every command must not wipe it.
	private var commandFailure: String?
	/// A read's failure clears with the next read that works.
	private var readFailure: String?
	private var expansion: ExpansionChoice = .followNextUp
	/// Whether the inbox is on screen; the controller says so.
	private var isPresented = false

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

	/// The list as drawn: everything, most pressing first, or one thread's
	/// items under a filter.
	public var visibleItems: [InboxMessage] {
		let items = inbox?.items ?? []
		guard let filter else { return items }
		return items.filter { $0.threadId == filter.threadId }
	}

	// MARK: Expansion

	/// The expanded item, if any.
	public var expanded: Expanded? {
		switch expansion {
		case .followNextUp:
			return inbox?.nextUp.map { .card($0.id) }
		case .chosen(let chosen):
			return chosen
		case .collapsed:
			return nil
		}
	}

	public func isExpanded(_ messageId: String) -> Bool {
		expanded?.messageId == messageId
	}

	/// A click on a row expands it and collapses the one that was open; a
	/// click on the expanded card's header collapses it.
	public func toggleExpansion(of messageId: String) {
		expansion = isExpanded(messageId) ? .collapsed : .chosen(.card(messageId))
	}

	/// The inbox opened or closed. Opening pins the default card: from then
	/// on only a click or the expanded item closing moves it, so a prompt
	/// that arrives (or a waiter that lapses) never swaps the card you are
	/// reading, and ↩ answers what you read. Closing lets the next opening
	/// start from the next-up item again, unfiltered.
	public func presentationChanged(isPresented: Bool) {
		self.isPresented = isPresented
		if isPresented {
			pinDefaultExpansion()
		} else {
			resetExpansion()
		}
	}

	/// The default card while hidden: the next-up item.
	public func resetExpansion() {
		expansion = .followNextUp
		filter = nil
		selection = []
	}

	private func pinDefaultExpansion() {
		guard isPresented, expansion == .followNextUp, let next = nextUpInView else { return }
		expansion = .chosen(.card(next.id))
	}

	/// After a read: an expanded item that left the list (answered,
	/// dismissed, withdrawn) hands over to the next-up item, or, when nothing
	/// waits, to the first row, so you can keep going. Under a filter both
	/// come from the filtered thread.
	private func keepExpansionCurrent() {
		if case .chosen(let chosen) = expansion,
			!(inbox?.items.contains { $0.id == chosen.messageId } ?? false)
		{
			if isPresented, let next = nextUpInView ?? visibleItems.first {
				expansion = .chosen(.card(next.id))
			} else {
				expansion = .followNextUp
			}
		}
		pinDefaultExpansion()
	}

	/// The next-up item if the list shows it; under a filter, the filtered
	/// thread's most pressing item instead (the list's first).
	private var nextUpInView: InboxMessage? {
		guard let next = inbox?.nextUp else { return filter == nil ? nil : visibleItems.first }
		return filter == nil || visibleItems.contains { $0.id == next.id } ? next : visibleItems.first
	}

	// MARK: Filter and selection

	public func clearFilter() {
		filter = nil
	}

	/// The filtered thread's items still open, counted live.
	public var filterOpenCount: Int {
		guard let filter else { return 0 }
		return (inbox?.items ?? []).filter { $0.threadId == filter.threadId }.count
	}

	/// Close, once the filtered thread has nothing open: the banner's button.
	public func closeFilteredThread() async {
		guard let filter, filterOpenCount == 0 else { return }
		if let failure = await perform({ try await $0.closeThread(threadId: filter.threadId) }) {
			commandFailure = failure
		} else {
			commandFailure = nil
			self.filter = nil
		}
		requestRead()
	}

	/// Bulk Dismiss never takes an `action_required` message: only Done
	/// closes those.
	public func canSelect(_ message: InboxMessage) -> Bool {
		message.messageType != .actionRequired
	}

	public func toggleSelection(of message: InboxMessage) {
		guard canSelect(message) else { return }
		if selection.contains(message.id) {
			selection.remove(message.id)
		} else {
			selection.insert(message.id)
		}
	}

	public func clearSelection() {
		selection = []
	}

	/// Dismisses each picked row as if it were dismissed alone (its agent
	/// hears a plain `dismissed`), and reports every failure together:
	/// "2 dismissed · 1 already answered".
	public func dismissSelected() async {
		let picked = (inbox?.items ?? []).filter { selection.contains($0.id) && canSelect($0) }
		selection = []
		var dismissed = 0
		var failures: [String: Int] = [:]
		for message in picked {
			if let failure = await perform({ try await $0.dismiss(messageId: message.id) }) {
				failures[failure.prefix(1).lowercased() + failure.dropFirst(), default: 0] += 1
			} else {
				dismissed += 1
			}
		}
		if failures.isEmpty {
			commandFailure = nil
		} else {
			let parts = ["\(dismissed) dismissed"] + failures.sorted { $0.key < $1.key }.map { "\($0.value) \($0.key)" }
			commandFailure = parts.joined(separator: " · ")
		}
		requestRead()
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
				keepExpansionCurrent()
				selection = selection.filter { id in inbox?.items.contains { $0.id == id } ?? false }
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

	/// Close on a message's thread. A thread closes only when nothing in it
	/// is open, so while items are open this narrows the list to them
	/// ("N items still open") and closes nothing; there is no close-and-
	/// dismiss. Snoozed items the list does not show make the hub refuse
	/// too, and that refusal is worded in the foot.
	public func closeThread(of message: InboxMessage) async -> CloseOutcome {
		let open = (inbox?.items ?? []).filter { $0.threadId == message.threadId }
		guard open.isEmpty else {
			filter = GroupFilter(threadId: message.threadId, title: message.group?.threadTitle ?? "this thread")
			return .stillOpen(open.count)
		}
		await run { try await $0.closeThread(threadId: message.threadId) }
		return commandFailure == nil ? .closed : .failed
	}

	/// Reopens a closed topic or thread (History and Settings, PR 5).
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
		commandFailure = await perform(command)
		requestRead()
	}

	/// Runs a command; its failure in words, or `nil`.
	private func perform(_ command: @Sendable (any InboxService) async throws -> Void) async -> String? {
		do {
			try await command(service)
			return nil
		} catch {
			return Self.words(for: error)
		}
	}

	/// A failure in words for the popover's foot.
	public static func words(for error: any Error) -> String {
		switch error as? GentboxError {
		case .AlreadyClosed(let state, let rescindReason):
			switch state {
			case .answered: return "Already answered"
			// "Withdrawn" only when the agent asked; the hub's own clean-up is not a withdrawal.
			case .rescinded where rescindReason == .handledInSession: return "Handled in the session"
			case .rescinded: return "Withdrawn by the agent"
			case .expired: return "Expired before your answer"
			default: return "Already closed"
			}
		case .NotFound: return "No longer there"
		case .HasOpenMessages(let count): return count == 1 ? "1 item still open" : "\(count) items still open"
		case .GeneralTopic: return "General cannot be closed"
		case .Refused(let message): return message
		case .Stopped: return "Gentbox has stopped"
		case .AnotherHubRunning(let message), .CannotStart(let message), .Internal(let message):
			return message
		case nil: return error.localizedDescription
		}
	}
}
