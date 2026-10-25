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
/// exactly one while anything is listed); a filter to one thread's open items; the rows picked for a
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

	/// The reply being written on the expanded card. It follows the card to
	/// a superseding message that still takes a reply.
	public struct ReplyDraft: Equatable, Sendable {
		public let messageId: String
		public var text: String
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
	/// The message the reply field is open on. Kept apart from the text, so
	/// a view that only shows where the field is open is not told of each
	/// keystroke.
	public private(set) var replyingTo: String?
	/// The reply field's text, while it shows.
	private var replyText = ""
	/// The reply field's message and text, while it shows.
	public var replyDraft: ReplyDraft? {
		replyingTo.map { ReplyDraft(messageId: $0, text: replyText) }
	}
	/// Messages an answer, a snooze or a dismiss took out of the list at
	/// once, before the hub confirmed: they come back if it refuses. The
	/// list never waits on the hub to respond to a click.
	private var optimisticallyRemoved: Set<String> = []
	/// Removals the hub accepted, with the reads started by then.
	@ObservationIgnored private var confirmedRemovals: [String: Int] = [:]
	@ObservationIgnored private var readsStarted = 0

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
		liveItems.isEmpty
	}

	/// The list as drawn: everything, most pressing first, or one thread's
	/// items under a filter.
	public var visibleItems: [InboxMessage] {
		let items = liveItems
		guard let filter else { return items }
		return items.filter { $0.threadId == filter.threadId }
	}

	// MARK: Expansion

	/// The expanded item: one whenever anything is listed, none otherwise.
	public var expanded: Expanded? {
		switch expansion {
		case .followNextUp:
			return (liveNextUp ?? visibleItems.first).map { .card($0.id) }
		case .chosen(let chosen):
			return chosen
		}
	}

	public func isExpanded(_ messageId: String) -> Bool {
		expanded?.messageId == messageId
	}

	/// A click on a row expands it and collapses the one that was open, and
	/// an unsent reply goes. The expanded card stays: one is always open.
	public func expand(_ messageId: String) {
		guard !isExpanded(messageId) else { return }
		open(messageId)
		replyingTo = nil
	}

	/// The card shows `messageId`; a card you read is never one picked for
	/// a bulk Dismiss.
	private func open(_ messageId: String) {
		expansion = .chosen(.card(messageId))
		selection.remove(messageId)
	}

	/// A click on the expanded card's header: its Open, if it offers one,
	/// opens the conversation. The card stays expanded.
	public func openConversation(of message: InboxMessage) async {
		guard let open = message.offeredActions.first(where: { $0.kind == .open }) else { return }
		await press(open, on: message)
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
		replyingTo = nil
		filter = nil
		selection = []
	}

	private func pinDefaultExpansion() {
		guard isPresented, expansion == .followNextUp, let next = nextUpInView ?? visibleItems.first else { return }
		open(next.id)
	}

	/// After a read: an expanded item that a newer message superseded stays
	/// expanded as that message, with its reply draft if it still takes a
	/// reply: the card never switches to something you did not pick, so ↩
	/// answers what you read. One that left the list otherwise (answered,
	/// dismissed, withdrawn) hands over to the next-up item, or, when nothing
	/// waits, to the first row, so you can keep going. Under a filter both
	/// come from the filtered thread.
	private func keepExpansionCurrent() {
		if case .chosen(let chosen) = expansion,
			!liveItems.contains(where: { $0.id == chosen.messageId })
		{
			if let successor = liveItems.first(where: { $0.supersedes.contains(chosen.messageId) }) {
				open(successor.id)
				carryReplyDraft(to: successor)
			} else {
				replyingTo = nil
				if isPresented, let next = nextUpInView ?? visibleItems.first {
					open(next.id)
				} else {
					expansion = .followNextUp
				}
			}
		}
		pinDefaultExpansion()
	}

	private func carryReplyDraft(to successor: InboxMessage) {
		guard let replyingTo, successor.supersedes.contains(replyingTo) else { return }
		let takesReply = successor.offeredActions.contains { $0.kind == .reply }
		self.replyingTo = takesReply ? successor.id : nil
	}

	// MARK: Reply

	/// R on the expanded card: the reply field opens, empty.
	public func startReply(on messageId: String) {
		replyText = ""
		replyingTo = messageId
	}

	public func updateReply(_ text: String) {
		guard replyingTo != nil else { return }
		replyText = text
	}

	/// Cancel, or the reply was sent.
	public func endReply() {
		replyingTo = nil
	}

	/// The next-up item if the list shows it; under a filter, the filtered
	/// thread's most pressing item instead (the list's first).
	private var nextUpInView: InboxMessage? {
		guard let next = liveNextUp else { return filter == nil ? nil : visibleItems.first }
		return filter == nil || visibleItems.contains { $0.id == next.id } ? next : visibleItems.first
	}

	// MARK: Filter and selection

	public func clearFilter() {
		filter = nil
	}

	/// The filtered thread's items still open, counted live.
	public var filterOpenCount: Int {
		guard let filter else { return 0 }
		return liveItems.filter { $0.threadId == filter.threadId }.count
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
		let picked = liveItems.filter { selection.contains($0.id) && canSelect($0) }
		selection = []
		takeOutOfTheList(picked.map(\.id))
		var dismissed = 0
		var failures: [String: Int] = [:]
		for message in picked {
			if let failure = await perform({ try await $0.dismiss(messageId: message.id) }) {
				optimisticallyRemoved.remove(message.id)
				failures[failure.prefix(1).lowercased() + failure.dropFirst(), default: 0] += 1
			} else {
				confirmedRemovals[message.id] = readsStarted
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
			readsStarted += 1
			let read = readsStarted
			do {
				inbox = try await service.inbox()
				readFailure = nil
				// The hub has the last word: what it no longer lists is gone,
				// and a read that began after a command succeeded shows the
				// message as the hub keeps it.
				let listed = Set(inbox?.items.map(\.id) ?? [])
				let settled = confirmedRemovals.filter { $0.value < read }.map(\.key)
				optimisticallyRemoved.subtract(settled)
				settled.forEach { confirmedRemovals[$0] = nil }
				optimisticallyRemoved.formIntersection(listed)
				keepExpansionCurrent()
				selection = selection.filter { listed.contains($0) && !optimisticallyRemoved.contains($0) }
			} catch {
				readFailure = Self.words(for: error)
			}
		} while isReadAgainNeeded
		isReading = false
	}

	// MARK: Commands

	/// A button on a message. An answer takes it out of the list at once;
	/// Defer and Open keep it.
	public func press(_ action: InboxAction, on message: InboxMessage, reply: String? = nil) async {
		let command: @Sendable (any InboxService) async throws -> Void = {
			try await $0.takeAction(messageId: message.id, actionId: action.id, value: reply)
		}
		if Self.isAnswer(action) {
			await runRemoving(message, command)
		} else {
			await run(command)
		}
	}

	/// What a click or a key does before the command's task runs: an answer,
	/// a snooze or a dismiss (`action` nil) takes the item out in this frame,
	/// not a run-loop turn later. The command that follows does the rest.
	public func takeOutAhead(_ message: InboxMessage, for action: InboxAction? = nil) {
		guard action.map(Self.isAnswer) ?? true else { return }
		takeOutOfTheList([message.id])
	}

	/// Whether `action` settles its item (an answer) rather than leaving it
	/// open, as a deferral does with Done to come.
	public static func isAnswer(_ action: InboxAction) -> Bool {
		switch action.kind {
		case .approve, .deny, .choice, .reply, .done: true
		default: false
		}
	}

	/// Remind me later; never reaches the agent. Out of the list at once.
	public func snooze(_ message: InboxMessage, minutes: UInt32 = 15) async {
		await runRemoving(message) { try await $0.snooze(messageId: message.id, minutes: minutes) }
	}

	/// Clears a message without answering it. Out of the list at once.
	public func dismiss(_ message: InboxMessage) async {
		await runRemoving(message) { try await $0.dismiss(messageId: message.id) }
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
		let open = liveItems.filter { $0.threadId == message.threadId }
		guard open.isEmpty else {
			filter = GroupFilter(threadId: message.threadId, title: message.group?.threadTitle ?? "this thread")
			// A card the filter hides would still be the one keys act on:
			// the expansion moves to the filtered thread's first item.
			if let expanded, !open.contains(where: { $0.id == expanded.messageId }) {
				replyingTo = nil
				self.open(nextUpInView?.id ?? open[0].id)
			}
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

	/// Takes the message out of the list in this frame, then runs the
	/// command; a refusal puts it back.
	private func runRemoving(_ message: InboxMessage, _ command: @Sendable (any InboxService) async throws -> Void) async {
		takeOutOfTheList([message.id])
		commandFailure = await perform(command)
		if commandFailure != nil {
			optimisticallyRemoved.remove(message.id)
			keepExpansionCurrent()
		} else {
			confirmedRemovals[message.id] = readsStarted
		}
		requestRead()
	}

	private func takeOutOfTheList(_ ids: [String]) {
		optimisticallyRemoved.formUnion(ids)
		selection.subtract(ids)
		keepExpansionCurrent()
	}

	/// The listed messages, less those taken out ahead of the hub.
	private var liveItems: [InboxMessage] {
		(inbox?.items ?? []).filter { !optimisticallyRemoved.contains($0.id) }
	}

	private var liveNextUp: InboxMessage? {
		inbox?.nextUp.flatMap { optimisticallyRemoved.contains($0.id) ? nil : $0 }
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
