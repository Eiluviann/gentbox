import Foundation
import Testing
@testable import GentboxCore

/// Counts reads and records commands; `inbox()` returns `next`.
final class FakeService: InboxService, @unchecked Sendable {
	var next = makeInbox()
	var reads = 0
	var commands: [String] = []
	var failure: GentboxError?
	/// Failures for one message only (bulk Dismiss).
	var failures: [String: GentboxError] = [:]
	/// Holds every command until `release()`, as a slow hub would.
	var holdsCommands = false
	private var held: [CheckedContinuation<Void, Never>] = []
	/// Commands run off the test's executor: recording one and parking it
	/// must be one step, or a `release()` between them leaves it parked.
	private let lock = NSLock()

	/// Lets every held command finish, and any that comes after.
	func release() {
		let waiting = lock.withLock {
			holdsCommands = false
			defer { held = [] }
			return held
		}
		waiting.forEach { $0.resume() }
	}

	func inbox() async throws -> Inbox {
		reads += 1
		return next
	}

	private func record(_ command: String) async throws {
		let holds = lock.withLock {
			commands.append(command)
			return holdsCommands
		}
		if holds {
			await withCheckedContinuation { continuation in
				let isReleased = lock.withLock {
					guard holdsCommands else { return true }
					held.append(continuation)
					return false
				}
				if isReleased { continuation.resume() }
			}
		}
		if let failure { throw failure }
	}

	func takeAction(messageId: String, actionId: String, value: String?) async throws {
		try await record("action \(messageId) \(actionId) \(value ?? "-")")
	}
	func snooze(messageId: String, minutes: UInt32?) async throws { try await record("snooze \(messageId) \(minutes ?? 0)") }
	func dismiss(messageId: String) async throws {
		try await record("dismiss \(messageId)")
		if let failure = failures[messageId] { throw failure }
	}
	func muteTopic(topicId: String, isMuted: Bool) async throws { try await record("mute topic \(topicId) \(isMuted)") }
	func muteThread(threadId: String, isMuted: Bool) async throws { try await record("mute thread \(threadId) \(isMuted)") }
	func markTopicRead(topicId: String) async throws { try await record("read topic \(topicId)") }
	func markThreadRead(threadId: String) async throws { try await record("read thread \(threadId)") }
	func closeTopic(topicId: String) async throws { try await record("close topic \(topicId)") }
	func reopenTopic(topicId: String) async throws { try await record("reopen topic \(topicId)") }
	func closeThread(threadId: String) async throws { try await record("close thread \(threadId)") }
	func reopenThread(threadId: String) async throws { try await record("reopen thread \(threadId)") }
	func moveThread(threadId: String, topicId: String) async throws { try await record("move \(threadId) \(topicId)") }
}

@Suite("Inbox model")
@MainActor
struct InboxModelTests {
	@Test func readsAndExposesTheMenuBarState() async {
		let service = FakeService()
		service.next = makeInbox(waiting: 2)
		let model = InboxModel(service: service)
		#expect(model.isEmpty)
		await model.readUntilCurrent()
		#expect(model.waitingCount == 2)
		#expect(service.reads == 1)
	}

	@Test func eventsDuringAReadCostOneMoreReadNotOnePerEvent() async {
		let service = FakeService()
		let model = InboxModel(service: service)
		model.requestRead()
		for _ in 0..<10 {
			model.handle(event: .topicChanged(topicId: "t1"))
		}
		await model.readUntilCurrent()
		#expect(service.reads <= 3)
	}

	@Test func commandsReachTheHubAndFailuresAreWorded() async {
		let service = FakeService()
		let model = InboxModel(service: service)
		let prompt = message(id: "m7", type: .decisionRequired, blocking: .blocking)
		await model.press(action("allow", .approve), on: prompt)
		await model.setThreadMuted("th1", true)
		await model.setTopicMuted("t1", false)
		#expect(service.commands == ["action m7 allow -", "mute thread th1 true", "mute topic t1 false"])
		#expect(model.failure == nil)

		service.failure = .AlreadyClosed(state: .answered, rescindReason: nil)
		await model.press(action("deny", .deny), on: prompt)
		// The read that follows every command must not wipe it.
		await model.readUntilCurrent()
		#expect(model.failure == "Already answered")

		// The next command that works clears it.
		service.failure = nil
		await model.snooze(prompt)
		await model.readUntilCurrent()
		#expect(model.failure == nil)
	}

	@Test func aLateAnswerSaysWhoClosedTheItem() {
		#expect(
			InboxModel.words(for: GentboxError.AlreadyClosed(state: .rescinded, rescindReason: .handledInSession))
				== "Handled in the session",
		)
		for reason in [RescindReason.resolvedElsewhere, .obsolete, .superseded] {
			#expect(
				InboxModel.words(for: GentboxError.AlreadyClosed(state: .rescinded, rescindReason: reason))
					== "Withdrawn by the agent",
			)
		}
	}

	@Test func theNextUpItemIsExpandedByDefault() async {
		let (model, _) = await accordion()
		#expect(model.expanded == .card("prompt"))
		#expect(model.isExpanded("prompt"))
		#expect(!model.isExpanded("done"))
	}

	@Test func oneItemIsExpandedAtATime() async {
		let (model, _) = await accordion()
		model.toggleExpansion(of: "done")
		#expect(model.expanded == .card("done"))
		#expect(!model.isExpanded("prompt"))
		model.toggleExpansion(of: "note")
		#expect(model.expanded == .card("note"))
		#expect(!model.isExpanded("done"))
	}

	@Test func collapsingTheExpandedItemLeavesNothingExpanded() async {
		let (model, service) = await accordion()
		model.toggleExpansion(of: "prompt")
		#expect(model.expanded == nil)
		// A refresh does not undo your choice.
		await model.readUntilCurrent()
		#expect(model.expanded == nil)
		// Closing the inbox starts again from the next-up item.
		model.resetExpansion()
		#expect(model.expanded == .card("prompt"))
		_ = service
	}

	@Test func whenTheExpandedItemClosesTheNextOneExpands() async {
		let (model, service) = await accordion()
		model.toggleExpansion(of: "done")
		let next = message(id: "next", type: .decisionRequired, blocking: .blocking)
		service.next = makeInbox(nextUp: next, items: [next, message(id: "note")])
		await model.readUntilCurrent()
		#expect(model.expanded == .card("next"))
		// Nothing waiting: the first row expands, so you can keep going.
		service.next = makeInbox(nextUp: nil, items: [message(id: "note")])
		await model.readUntilCurrent()
		#expect(model.expanded == .card("note"))
		// Nothing listed: nothing expands.
		service.next = makeInbox(nextUp: nil, items: [])
		await model.readUntilCurrent()
		#expect(model.expanded == nil)
	}

	@Test func theDefaultCardIsPinnedOnceShown() async {
		let (model, service) = await accordion()
		#expect(model.expanded == .card("prompt"))
		// A more pressing prompt arrives (or this one's waiter lapses): the card
		// you are reading stays, so ↩ answers what you read.
		let urgent = message(id: "urgent", type: .decisionRequired, blocking: .blocking)
		service.next = makeInbox(nextUp: urgent, items: [urgent, message(id: "prompt", type: .decisionRequired, blocking: .blocking), message(id: "done")])
		await model.readUntilCurrent()
		#expect(model.expanded == .card("prompt"))
		// Closing the inbox lets the next opening start from the next-up item.
		model.presentationChanged(isPresented: false)
		model.presentationChanged(isPresented: true)
		#expect(model.expanded == .card("urgent"))
	}

	@Test func whileHiddenTheDefaultFollowsTheNextUpItem() async {
		let prompt = message(id: "prompt", type: .decisionRequired, blocking: .blocking)
		let service = FakeService()
		service.next = makeInbox(nextUp: prompt, items: [prompt])
		let model = InboxModel(service: service)
		await model.readUntilCurrent()
		let urgent = message(id: "urgent", type: .decisionRequired, blocking: .blocking)
		service.next = makeInbox(nextUp: urgent, items: [urgent, prompt])
		await model.readUntilCurrent()
		#expect(model.expanded == .card("urgent"))
	}

	@Test func theExpansionSurvivesARefresh() async {
		let (model, service) = await accordion()
		model.toggleExpansion(of: "done")
		// Re-read in another order, with a new next-up item.
		let urgent = message(id: "urgent", type: .decisionRequired, blocking: .blocking)
		service.next = makeInbox(
			nextUp: urgent,
			items: [urgent, message(id: "note"), message(id: "done"), message(id: "prompt", type: .decisionRequired, blocking: .blocking)],
		)
		await model.readUntilCurrent()
		#expect(model.expanded == .card("done"))
	}

	@Test func closeLeadsToTheOpenItemsInsteadOfClosing() async {
		let (model, service) = await accordion()
		let outcome = await model.closeThread(of: message(id: "done"))
		#expect(outcome == .stillOpen(3))
		#expect(model.filter == InboxModel.GroupFilter(threadId: "th1", title: "Phase 3"))
		#expect(model.filterOpenCount == 3)
		#expect(model.visibleItems.map(\.id) == ["prompt", "done", "note"])
		#expect(!service.commands.contains { $0.hasPrefix("close") })
		model.clearFilter()
		#expect(model.filter == nil)
	}

	/// A click's answer leaves the list before any task runs; Defer stays.
	@Test func takingOutAheadIsImmediateForAnswersOnly() async {
		let (model, _) = await accordion()
		let prompt = message(id: "prompt", type: .decisionRequired, blocking: .blocking)
		model.takeOutAhead(prompt, for: action("later", .defer))
		#expect(model.visibleItems.contains { $0.id == "prompt" })
		model.takeOutAhead(prompt, for: action("allow", .approve))
		#expect(!model.visibleItems.contains { $0.id == "prompt" })
		model.takeOutAhead(message(id: "done"))
		#expect(model.visibleItems.map(\.id) == ["note"])
	}

	/// An answer settles the item; a deferral leaves it open with Done to
	/// come, so the card keeps its keys for it.
	@Test func onlyAnswersAreAnswers() {
		for kind in [ActionKind.approve, .deny, .choice, .reply, .done] {
			#expect(InboxModel.isAnswer(action("a", kind)), "\(kind)")
		}
		#expect(!InboxModel.isAnswer(action("later", .defer)))
	}

	/// Close on another thread's row filters the list to that thread: the
	/// card the filter hides is no longer the expanded one, so no key acts
	/// on it unseen.
	@Test func aFilterMovesTheExpansionIntoTheFilteredThread() async {
		let prompt = message(id: "prompt", type: .decisionRequired, blocking: .blocking, threadId: "th1")
		let other = message(id: "other", threadId: "th2")
		let service = FakeService()
		service.next = makeInbox(nextUp: prompt, items: [prompt, other, message(id: "more", threadId: "th2")])
		let model = InboxModel(service: service)
		await model.readUntilCurrent()
		model.presentationChanged(isPresented: true)
		model.startReply(on: "prompt")
		#expect(model.expanded == .card("prompt"))
		let outcome = await model.closeThread(of: other)
		#expect(outcome == .stillOpen(2))
		#expect(model.expanded == .card("other"))
		#expect(model.replyDraft == nil)
		#expect(!service.commands.contains { $0.hasPrefix("action") })
	}

	@Test func theOpenCountFallsLiveAndCloseWorksAtZero() async {
		let (model, service) = await accordion()
		_ = await model.closeThread(of: message(id: "done"))
		service.next = makeInbox(items: [message(id: "note")])
		await model.readUntilCurrent()
		#expect(model.filterOpenCount == 1)
		service.next = makeInbox(items: [])
		await model.readUntilCurrent()
		#expect(model.filterOpenCount == 0)
		await model.closeFilteredThread()
		#expect(service.commands.last == "close thread th1")
		#expect(model.filter == nil)
	}

	@Test func underAFilterTheHandOverStaysInTheFilteredThread() async {
		let prompt = message(id: "prompt", type: .decisionRequired, blocking: .blocking, threadId: "th2")
		let first = message(id: "first")
		let second = message(id: "second")
		let service = FakeService()
		service.next = makeInbox(nextUp: prompt, items: [prompt, first, second])
		let model = InboxModel(service: service)
		await model.readUntilCurrent()
		model.presentationChanged(isPresented: true)
		_ = await model.closeThread(of: first)
		#expect(model.filter?.threadId == "th1")
		// The filter hid the prompt's card: the expansion moved into the thread.
		#expect(model.expanded == .card("first"))

		service.next = makeInbox(nextUp: prompt, items: [prompt, second])
		await model.readUntilCurrent()
		#expect(model.expanded == .card("second"))
	}

	@Test func aSupersededCardFollowsItsReplacementWithItsReplyDraft() async {
		let reply = action("reply", .reply)
		let old = message(id: "old", type: .decisionRequired, blocking: .blocking, actions: [reply])
		let other = message(id: "other", type: .decisionRequired, blocking: .blocking, actions: [action("allow", .approve)])
		let service = FakeService()
		service.next = makeInbox(nextUp: other, items: [other, old])
		let model = InboxModel(service: service)
		await model.readUntilCurrent()
		model.presentationChanged(isPresented: true)
		model.toggleExpansion(of: "old")
		model.startReply(on: "old")
		model.updateReply("use the staging key")

		let replacement = message(
			id: "new", type: .decisionRequired, blocking: .blocking, actions: [reply], supersedes: ["mid", "old"],
		)
		service.next = makeInbox(nextUp: other, items: [other, replacement])
		await model.readUntilCurrent()
		// Never the next-up prompt: ↩ answers the card you were reading.
		#expect(model.expanded == .card("new"))
		#expect(model.replyDraft == InboxModel.ReplyDraft(messageId: "new", text: "use the staging key"))

		// A replacement that takes no reply keeps the card, not the draft.
		let final = message(id: "final", type: .decisionRequired, blocking: .blocking, supersedes: ["new"])
		service.next = makeInbox(nextUp: other, items: [other, final])
		await model.readUntilCurrent()
		#expect(model.expanded == .card("final"))
		#expect(model.replyDraft == nil)
	}

	@Test func aDismissTakesTheItemOutBeforeTheHubAnswers() async {
		let (model, service) = await accordion()
		let done = message(id: "done")
		service.holdsCommands = true
		let dismissing = Task { await model.dismiss(done) }
		while service.commands.isEmpty { await Task.yield() }
		// The hub has not answered yet; the row is gone already.
		#expect(!model.visibleItems.contains { $0.id == "done" })

		service.next = makeInbox(nextUp: model.inbox?.nextUp, items: model.visibleItems)
		service.release()
		await dismissing.value
		await model.readUntilCurrent()
		#expect(!model.visibleItems.contains { $0.id == "done" })
	}

	@Test func aRefusedAnswerPutsTheItemBack() async {
		let (model, service) = await accordion()
		let prompt = message(id: "prompt", type: .decisionRequired, blocking: .blocking)
		service.failure = .AlreadyClosed(state: .expired, rescindReason: nil)
		await model.press(action("allow", .approve), on: prompt)
		#expect(model.visibleItems.contains { $0.id == "prompt" })
		#expect(model.failure == "Expired before your answer")
	}

	@Test func deferKeepsTheItemListed() async {
		let (model, service) = await accordion()
		let prompt = message(id: "prompt", type: .decisionRequired, blocking: .blocking)
		service.holdsCommands = true
		let deferring = Task { await model.press(action("later", .defer), on: prompt) }
		while service.commands.isEmpty { await Task.yield() }
		#expect(model.visibleItems.contains { $0.id == "prompt" })
		service.release()
		await deferring.value
	}

	@Test func anItemTheHubStillListsAfterTheCommandShowsAgain() async {
		let (model, _) = await accordion()
		await model.dismiss(message(id: "note", type: .generic))
		// The hub kept it: the read after the command has the last word.
		await model.readUntilCurrent()
		#expect(model.visibleItems.contains { $0.id == "note" })
	}

	@Test func bulkDismissDismissesEachItemAndSkipsActionRequired() async {
		let (model, service) = await accordion()
		let task = message(id: "task", type: .actionRequired)
		#expect(!model.canSelect(task))
		model.toggleSelection(of: task)
		model.toggleSelection(of: message(id: "done"))
		model.toggleSelection(of: message(id: "note"))
		#expect(model.selection == ["done", "note"])
		await model.dismissSelected()
		#expect(service.commands == ["dismiss done", "dismiss note"])
		#expect(model.selection.isEmpty)
	}

	@Test func bulkDismissReportsEveryFailure() async {
		let (model, service) = await accordion()
		service.failures = ["done": .AlreadyClosed(state: .answered, rescindReason: nil)]
		model.toggleSelection(of: message(id: "done"))
		model.toggleSelection(of: message(id: "note"))
		await model.dismissSelected()
		await model.readUntilCurrent()
		#expect(model.failure == "1 dismissed · 1 already answered")
	}

	/// A blocked prompt (next up), a completion and a note, all in one thread.
	private func accordion() async -> (InboxModel, FakeService) {
		let prompt = message(id: "prompt", type: .decisionRequired, blocking: .blocking)
		let service = FakeService()
		service.next = makeInbox(nextUp: prompt, items: [prompt, message(id: "done"), message(id: "note", type: .generic)])
		let model = InboxModel(service: service)
		await model.readUntilCurrent()
		model.presentationChanged(isPresented: true)
		return (model, service)
	}

	@Test func reopenPicksTheGroupKind() async {
		let service = FakeService()
		let model = InboxModel(service: service)
		let closed = ClosedGroup(kind: .thread, id: "th9", title: "v1.0", topicTitle: "Gentbox", closedAt: Date())
		await model.reopen(closed)
		#expect(service.commands == ["reopen thread th9"])
	}
}
