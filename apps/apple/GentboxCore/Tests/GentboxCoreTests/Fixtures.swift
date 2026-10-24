import Foundation
@testable import GentboxCore

/// A message row with defaults for every field a test does not care about.
func message(
	id: String = "m1",
	title: String = "Ingest tests pass",
	type: MessageType = .taskCompleted,
	blocking: Blocking = .nonBlocking,
	actions: [InboxAction] = [],
	asksPermission: Bool = false,
	conversationTitle: String? = "gentbox",
	receivedAt: Date = Date(timeIntervalSince1970: 1_000_000),
) -> InboxMessage {
	InboxMessage(
		id: id, title: title, summary: nil, body: nil, messageType: type, isUrgent: false,
		blocking: blocking, state: .open, agent: "claude-code", agentName: "Claude Code",
		conversationId: "session-1", conversationTitle: conversationTitle, topicId: "t1",
		threadId: "th1", receivedAt: receivedAt, hasLiveWaiter: false, offeredActions: actions,
		asksPermission: asksPermission, repeatCount: 0, stepCount: 0,
	)
}

func action(_ id: String, _ kind: ActionKind) -> InboxAction {
	InboxAction(id: id, label: id.capitalized, kind: kind, value: nil, placeholder: nil, isDestructive: false)
}

func makeInbox(waiting: UInt32 = 0, topics: [InboxTopic] = [], muted: [MutedGroup] = []) -> Inbox {
	Inbox(
		nextUp: nil, topics: topics, closed: [], muted: muted, moveTargets: [],
		waitingCount: waiting, hasUnread: false,
	)
}
