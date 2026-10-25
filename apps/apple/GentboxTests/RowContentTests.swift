import Foundation
import GentboxCore
import XCTest

/// A pick changes a row's tint and checkbox only, so the first pick, which
/// touches every row, lays none of them out.
@MainActor
final class RowContentTests: XCTestCase {
	private let message = InboxMessage(
		id: "m1", title: "Ingest tests pass", summary: nil, body: nil, messageType: .taskCompleted, isUrgent: false,
		blocking: .nonBlocking, state: .open, agent: "claude-code", agentName: "Claude Code",
		conversationId: "session-1", conversationTitle: nil, topicId: "t1", threadId: "th1",
		receivedAt: Date(timeIntervalSince1970: 1_000_000), hasLiveWaiter: false, offeredActions: [],
		asksPermission: false, repeatCount: 0, stepCount: 0, group: nil, supersedes: [],
	)

	private func row(
		isExpanded: Bool = false, isPicked: Bool = false, isSelecting: Bool = false, isFirst: Bool = false,
		title: String? = nil,
	) -> RowContent {
		let shown = title.map {
			InboxMessage(
				id: message.id, title: $0, summary: nil, body: nil, messageType: .taskCompleted, isUrgent: false,
				blocking: .nonBlocking, state: .open, agent: message.agent, agentName: message.agentName,
				conversationId: message.conversationId, conversationTitle: nil, topicId: message.topicId,
				threadId: message.threadId, receivedAt: message.receivedAt, hasLiveWaiter: false, offeredActions: [],
				asksPermission: false, repeatCount: 0, stepCount: 0, group: nil, supersedes: [],
			)
		} ?? message
		return RowContent(
			message: shown, isExpanded: isExpanded, isPicked: isPicked, isSelecting: isSelecting, isFirst: isFirst,
			isReplying: false, moveTargets: [],
		)
	}

	func testPickingAndTheStartOfPickingArePickOnlyChanges() {
		XCTAssertTrue(row(isSelecting: true).differsOnlyInPicking(from: row()))
		XCTAssertTrue(row(isPicked: true, isSelecting: true).differsOnlyInPicking(from: row(isSelecting: true)))
		XCTAssertTrue(row().differsOnlyInPicking(from: row(isPicked: true, isSelecting: true)))
	}

	func testAnyOtherChangeIsLaidOut() {
		XCTAssertFalse(row().differsOnlyInPicking(from: row()), "no change at all")
		XCTAssertFalse(row(isSelecting: true, title: "Edited").differsOnlyInPicking(from: row()))
		XCTAssertFalse(row(isExpanded: true, isSelecting: true).differsOnlyInPicking(from: row()))
		XCTAssertFalse(row(isSelecting: true, isFirst: true).differsOnlyInPicking(from: row()))
	}
}
