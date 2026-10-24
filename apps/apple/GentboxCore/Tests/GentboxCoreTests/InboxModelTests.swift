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

	func inbox() async throws -> Inbox {
		reads += 1
		return next
	}

	private func record(_ command: String) throws {
		commands.append(command)
		if let failure { throw failure }
	}

	func takeAction(messageId: String, actionId: String, value: String?) async throws {
		try record("action \(messageId) \(actionId) \(value ?? "-")")
	}
	func snooze(messageId: String, minutes: UInt32?) async throws { try record("snooze \(messageId) \(minutes ?? 0)") }
	func dismiss(messageId: String) async throws {
		try record("dismiss \(messageId)")
		if let failure = failures[messageId] { throw failure }
	}
	func muteTopic(topicId: String, isMuted: Bool) async throws { try record("mute topic \(topicId) \(isMuted)") }
	func muteThread(threadId: String, isMuted: Bool) async throws { try record("mute thread \(threadId) \(isMuted)") }
	func markTopicRead(topicId: String) async throws { try record("read topic \(topicId)") }
	func markThreadRead(threadId: String) async throws { try record("read thread \(threadId)") }
	func closeTopic(topicId: String) async throws { try record("close topic \(topicId)") }
	func reopenTopic(topicId: String) async throws { try record("reopen topic \(topicId)") }
	func closeThread(threadId: String) async throws { try record("close thread \(threadId)") }
	func reopenThread(threadId: String) async throws { try record("reopen thread \(threadId)") }
	func moveThread(threadId: String, topicId: String) async throws { try record("move \(threadId) \(topicId)") }
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

		service.failure = .AlreadyClosed(state: .answered)
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
		service.failures = ["done": .AlreadyClosed(state: .answered)]
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
