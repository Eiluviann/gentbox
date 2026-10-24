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
	group: GroupContext? = GroupContext(
		topicTitle: "Gentbox", topicIsGeneral: false, topicIsMuted: false,
		threadTitle: "Phase 3", threadIsMuted: false,
	),
	conversationTitle: String? = "gentbox",
	receivedAt: Date = Date(timeIntervalSince1970: 1_000_000),
	threadId: String = "th1",
	supersedes: [String] = [],
) -> InboxMessage {
	InboxMessage(
		id: id, title: title, summary: nil, body: nil, messageType: type, isUrgent: false,
		blocking: blocking, state: .open, agent: "claude-code", agentName: "Claude Code",
		conversationId: "session-1", conversationTitle: conversationTitle, topicId: "t1",
		threadId: threadId, receivedAt: receivedAt, hasLiveWaiter: false, offeredActions: actions,
		asksPermission: asksPermission, repeatCount: 0, stepCount: 0, group: group,
		supersedes: supersedes,
	)
}

func action(_ id: String, _ kind: ActionKind) -> InboxAction {
	InboxAction(id: id, label: id.capitalized, kind: kind, value: nil, placeholder: nil, isDestructive: false)
}

func makeInbox(
	waiting: UInt32 = 0,
	nextUp: InboxMessage? = nil,
	items: [InboxMessage] = [],
	topics: [InboxTopic] = [],
	muted: [MutedGroup] = [],
) -> Inbox {
	Inbox(
		nextUp: nextUp, items: items, topics: topics, closed: [], muted: muted, moveTargets: [],
		waitingCount: waiting, hasUnread: false,
	)
}
