import Foundation
import Testing
@testable import GentboxCore

/// Counts reads and records commands; `inbox()` returns `next`.
final class FakeService: InboxService, @unchecked Sendable {
	var next = makeInbox()
	var reads = 0
	var commands: [String] = []
	var failure: GentboxError?

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
	func dismiss(messageId: String) async throws { try record("dismiss \(messageId)") }
	func muteTopic(topicId: String, isMuted: Bool) async throws { try record("mute topic \(topicId) \(isMuted)") }
	func muteThread(threadId: String, isMuted: Bool) async throws { try record("mute thread \(threadId) \(isMuted)") }
	func markTopicRead(topicId: String) async throws { try record("read topic \(topicId)") }
	func markThreadRead(threadId: String) async throws { try record("read thread \(threadId)") }
	func closeTopic(topicId: String, dismissOpenMessages: Bool) async throws {
		try record("close topic \(topicId) \(dismissOpenMessages)")
	}
	func reopenTopic(topicId: String) async throws { try record("reopen topic \(topicId)") }
	func closeThread(threadId: String, dismissOpenMessages: Bool) async throws {
		try record("close thread \(threadId) \(dismissOpenMessages)")
	}
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
		await model.closeThread("th1", dismissingOpenMessages: true)
		#expect(service.commands == ["action m7 allow -", "mute thread th1 true", "close thread th1 true"])
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

	@Test func aPickedRowTakesTheCardUntilItLeavesTheList() async {
		let prompt = message(id: "prompt", type: .decisionRequired, blocking: .blocking)
		let done = message(id: "done")
		let service = FakeService()
		service.next = makeInbox(nextUp: prompt, items: [prompt, done])
		let model = InboxModel(service: service)
		await model.readUntilCurrent()
		#expect(model.card?.id == "prompt")
		#expect(model.list.map(\.id) == ["done"])

		model.selectedMessageId = "done"
		#expect(model.card?.id == "done")
		#expect(model.list.map(\.id) == ["prompt"])

		service.next = makeInbox(nextUp: prompt, items: [prompt])
		await model.readUntilCurrent()
		#expect(model.card?.id == "prompt")
		#expect(model.list.isEmpty)
	}

	@Test func reopenPicksTheGroupKind() async {
		let service = FakeService()
		let model = InboxModel(service: service)
		let closed = ClosedGroup(kind: .thread, id: "th9", title: "v1.0", topicTitle: "Gentbox", closedAt: Date())
		await model.reopen(closed)
		#expect(service.commands == ["reopen thread th9"])
	}
}
