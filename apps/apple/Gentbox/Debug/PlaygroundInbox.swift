#if DEBUG || GENTBOX_HOOKS
import Foundation
import GentboxCore

/// An inbox held in memory for the debug panel: messages added, changed,
/// replaced and withdrawn at will, so every motion of the inbox can be tried
/// without an agent. It answers the inbox's commands as the hub would,
/// closely enough for the list: an answer, a snooze or a dismiss takes a
/// message out; Open and Defer leave it.
actor PlaygroundInbox: InboxService {
	/// What the debug panel adds.
	enum Kind: String, CaseIterable, Sendable {
		case decision = "Decision (agent waiting)"
		case urgentDecision = "Urgent decision"
		case action = "Action required"
		case completed = "Task completed"
		case note = "Note"
	}

	/// Every id the playground makes starts so (messages, topics, threads),
	/// so a command reaches the playground or the hub by what it names.
	static let idPrefix = "playground-"

	private var items: [InboxMessage] = []
	private var serial = 0

	// MARK: The debug panel's changes

	/// A list with one of each kind, two of the commonest.
	func populate() {
		for kind in [Kind.decision, .completed, .action, .decision, .completed, .note] {
			_ = add(kind)
		}
	}

	func clear() {
		items = []
	}

	/// Adds a message of `kind`; its id.
	@discardableResult
	func add(_ kind: Kind) -> String {
		serial += 1
		let message = Self.message(kind, serial: serial, receivedAt: Date())
		items.append(message)
		return message.id
	}

	/// The agent withdraws the message: it leaves the list.
	func withdraw(_ messageId: String) {
		items.removeAll { $0.id == messageId }
	}

	/// The newest message, which the list shows last among its kind.
	var newestId: String? {
		items.max { $0.receivedAt < $1.receivedAt }?.id
	}

	/// A newer message from the same agent replaces this one in place.
	func supersede(_ messageId: String) {
		guard let index = items.firstIndex(where: { $0.id == messageId }) else { return }
		let old = items[index]
		serial += 1
		items[index] = Self.copy(
			of: old, id: "\(Self.idPrefix)\(serial)", title: Self.nextStep(of: old.title),
			summary: "Replaced \(Self.now): the agent sent a newer message.", supersedes: [old.id],
		)
	}

	/// The same message with other text, as when an agent edits it.
	func changeText(of messageId: String) {
		guard let index = items.firstIndex(where: { $0.id == messageId }) else { return }
		let old = items[index]
		let isLong = (old.summary?.count ?? 0) > 80
		items[index] = Self.copy(
			of: old, id: old.id, title: old.title,
			summary: isLong
				? "Short again."
				: "Edited \(Self.now). The agent rewrote this summary at length, so the card has to make room for a second and a third line of it.",
			supersedes: old.supersedes,
		)
	}

	/// An agent starts or stops waiting on a decision: its answers come and
	/// go with the waiter, as the hub offers them.
	func toggleWaiter(on messageId: String) {
		guard let index = items.firstIndex(where: { $0.id == messageId }) else { return }
		let old = items[index]
		guard old.messageType == .decisionRequired else { return }
		items[index] = Self.copy(of: old, hasLiveWaiter: !old.hasLiveWaiter)
	}

	// MARK: InboxService

	/// Urgent first, then what agents wait on, the most blocking first, then
	/// the rest, newest first.
	func inbox() -> Inbox {
		let ordered = items.enumerated().sorted { left, right in
			let (a, b) = (Self.rank(left.element), Self.rank(right.element))
			if a != b { return a < b }
			if left.element.receivedAt != right.element.receivedAt { return left.element.receivedAt > right.element.receivedAt }
			return left.offset < right.offset
		}.map(\.element)
		let waiting = ordered.filter(\.hasLiveWaiter)
		return Inbox(
			nextUp: waiting.first, items: ordered, topics: [], closed: [], muted: [], moveTargets: [],
			waitingCount: UInt32(waiting.count), hasUnread: !ordered.isEmpty,
		)
	}

	func takeAction(messageId: String, actionId: String, value _: String?) {
		guard let action = items.first(where: { $0.id == messageId })?.offeredActions.first(where: { $0.id == actionId }),
			[.approve, .deny, .choice, .reply, .done].contains(action.kind)
		else { return }
		withdraw(messageId)
	}

	func snooze(messageId: String, minutes _: UInt32?) {
		withdraw(messageId)
	}

	func dismiss(messageId: String) {
		withdraw(messageId)
	}

	// Muting, reading, closing and moving change nothing the playground shows.
	func muteTopic(topicId _: String, isMuted _: Bool) {}
	func muteThread(threadId _: String, isMuted _: Bool) {}
	func markTopicRead(topicId _: String) {}
	func markThreadRead(threadId _: String) {}
	func closeTopic(topicId _: String) {}
	func reopenTopic(topicId _: String) {}
	func closeThread(threadId _: String) {}
	func reopenThread(threadId _: String) {}
	func moveThread(threadId _: String, topicId _: String) {}

	// MARK: Messages

	private static var now: String {
		Date().formatted(date: .omitted, time: .standard)
	}

	private static func rank(_ message: InboxMessage) -> Int {
		if message.isUrgent { return 0 }
		guard message.hasLiveWaiter else { return 3 }
		return message.blocking == .blocking ? 1 : 2
	}

	private struct Template {
		let title: String
		let summary: String?
		let body: String?
		let topic: String
		let thread: String
	}

	private static let templates: [Kind: [Template]] = [
		.decision: [
			Template(title: "Allow Bash to run the ingest tests?", summary: nil, body: "pnpm --filter ingest test -- --run", topic: "Gentbox", thread: "Ingest auth"),
			Template(title: "Use WAL mode for the hub's database?", summary: "Faster concurrent reads; one more file beside the store.", body: "PRAGMA journal_mode = WAL;", topic: "Gentbox", thread: "Phase 3"),
			Template(title: "Delete the stale fixtures folder?", summary: "Nothing has read it since the move to the new loader.", body: "rm -rf tests/fixtures/legacy", topic: "Web", thread: "Housekeeping"),
		],
		.urgentDecision: [
			Template(title: "Force-push to main?", summary: "The rebase rewrote three published commits.", body: "git push --force-with-lease origin main", topic: "Gentbox", thread: "Release"),
		],
		.action: [
			Template(title: "Deploy 1 of 3 is waiting on review", summary: "staging-eu needs a reviewer before it rolls out", body: "deploy/staging-eu\nchecks: 12/12 green\nreview: pending\nrollout: paused", topic: "Infrastructure", thread: "Staging rollout"),
			Template(title: "Rotate the API token before Friday", summary: "The current token expires at the end of the week.", body: nil, topic: "Infrastructure", thread: "Secrets"),
		],
		.completed: [
			Template(title: "Store migrations run on start; 42 tests pass", summary: nil, body: nil, topic: "Gentbox", thread: "Phase 3"),
			Template(title: "Search filters persist in the URL; 18 tests pass", summary: "Filters survive reloads and shared links", body: nil, topic: "Web", thread: "Search filters"),
			Template(title: "Staging certificates rotated", summary: nil, body: nil, topic: "Infrastructure", thread: "Rotate staging certificates"),
			Template(title: "Bundle size down 12 kB", summary: nil, body: nil, topic: "Web", thread: "Housekeeping"),
		],
		.note: [
			Template(title: "Release notes drafted for v1.1", summary: nil, body: nil, topic: "Gentbox", thread: "v1.1"),
			Template(title: "Lint passes on the web app", summary: nil, body: nil, topic: "Web", thread: "Housekeeping"),
		],
	]

	private static func message(_ kind: Kind, serial: Int, receivedAt: Date) -> InboxMessage {
		let choices = templates[kind] ?? []
		let template = choices.isEmpty
			? Template(title: kind.rawValue, summary: nil, body: nil, topic: "Gentbox", thread: "Playground")
			: choices[serial % choices.count]
		let type: MessageType =
			switch kind {
			case .decision, .urgentDecision: .decisionRequired
			case .action: .actionRequired
			case .completed: .taskCompleted
			case .note: .generic
			}
		let blocking: Blocking =
			switch kind {
			case .decision, .urgentDecision: .blocking
			case .action: .deferrable
			case .completed, .note: .nonBlocking
			}
		let isWaiting = kind == .decision || kind == .urgentDecision
		return InboxMessage(
			id: "\(idPrefix)\(serial)", title: template.title, summary: template.summary, body: template.body,
			messageType: type, isUrgent: kind == .urgentDecision, blocking: blocking, state: .open,
			agent: serial % 3 == 0 ? "codex" : "claude-code", agentName: serial % 3 == 0 ? "Codex" : "Claude Code",
			conversationId: "\(idPrefix)conversation-\(template.thread)", conversationTitle: template.thread,
			topicId: "\(idPrefix)topic-\(template.topic)", threadId: "\(idPrefix)thread-\(template.thread)", receivedAt: receivedAt, hasLiveWaiter: isWaiting,
			offeredActions: actions(for: type, isWaiting: isWaiting), asksPermission: type == .decisionRequired,
			repeatCount: 0, stepCount: 0,
			group: GroupContext(
				topicTitle: template.topic, topicIsGeneral: false, topicIsMuted: false, threadTitle: template.thread,
				threadIsMuted: false,
			),
			supersedes: [],
		)
	}

	/// What the hub offers: a decision's answers only while its agent
	/// waits; Done on an action; Open on everything an agent sent.
	private static func actions(for type: MessageType, isWaiting: Bool) -> [InboxAction] {
		let open = InboxAction(id: "open", label: "Open", kind: .open, value: nil, placeholder: nil, isDestructive: false)
		switch type {
		case .decisionRequired:
			guard isWaiting else { return [open] }
			return [
				InboxAction(id: "allow", label: "Approve", kind: .approve, value: nil, placeholder: nil, isDestructive: false),
				InboxAction(id: "deny", label: "Deny", kind: .deny, value: nil, placeholder: nil, isDestructive: true),
				InboxAction(id: "reply", label: "Reply", kind: .reply, value: nil, placeholder: "Tell the agent what to do instead", isDestructive: false),
				open,
			]
		case .actionRequired:
			return [InboxAction(id: "done", label: "Done", kind: .done, value: nil, placeholder: nil, isDestructive: false), open]
		case .taskCompleted:
			return [open]
		case .generic:
			return []
		}
	}

	private static func copy(
		of message: InboxMessage, id: String? = nil, title: String? = nil, summary: String? = nil,
		hasLiveWaiter: Bool? = nil, supersedes: [String]? = nil,
	) -> InboxMessage {
		let isWaiting = hasLiveWaiter ?? message.hasLiveWaiter
		return InboxMessage(
			id: id ?? message.id, title: title ?? message.title, summary: summary ?? message.summary, body: message.body,
			messageType: message.messageType, isUrgent: message.isUrgent, blocking: message.blocking, state: message.state,
			agent: message.agent, agentName: message.agentName, conversationId: message.conversationId,
			conversationTitle: message.conversationTitle, topicId: message.topicId, threadId: message.threadId,
			receivedAt: message.receivedAt, hasLiveWaiter: isWaiting,
			offeredActions: actions(for: message.messageType, isWaiting: isWaiting), asksPermission: message.asksPermission,
			repeatCount: message.repeatCount, stepCount: message.stepCount, group: message.group,
			supersedes: supersedes ?? message.supersedes,
		)
	}

	/// "Deploy 1 of 3 …" becomes "Deploy 2 of 3 …"; any other title gets a
	/// step count.
	private static func nextStep(of title: String) -> String {
		if let range = title.range(of: #"\d+ of \d+"#, options: .regularExpression) {
			let parts = title[range].split(separator: " ")
			if let step = parts.first.flatMap({ Int($0) }), let total = parts.last.flatMap({ Int($0) }), total > 0 {
				return title.replacingCharacters(in: range, with: "\(step % total + 1) of \(total)")
			}
		}
		if let range = title.range(of: #" \(\d+\)$"#, options: .regularExpression),
			let step = Int(title[range].filter(\.isNumber))
		{
			return title.replacingCharacters(in: range, with: " (\(step + 1))")
		}
		return title + " (2)"
	}
}
#endif
