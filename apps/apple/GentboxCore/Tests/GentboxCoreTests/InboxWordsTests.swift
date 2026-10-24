import AppKit
import Foundation
import Testing
@testable import GentboxCore

@Suite("Inbox words")
struct InboxWordsTests {
	private let now = Date(timeIntervalSince1970: 2_000_000)

	@Test func timeAgo() {
		#expect(InboxWords.timeAgo(now.addingTimeInterval(-20), now: now) == "now")
		#expect(InboxWords.timeAgo(now.addingTimeInterval(-125), now: now) == "2 min ago")
		#expect(InboxWords.timeAgo(now.addingTimeInterval(-7300), now: now) == "2 h ago")
		#expect(InboxWords.timeAgo(now.addingTimeInterval(-3 * 86400), now: now) == "3 d ago")
		// A clock skewed ahead never reads as the future.
		#expect(InboxWords.timeAgo(now.addingTimeInterval(30), now: now) == "now")
	}

	@Test func breadcrumbIsTopicThenThread() {
		#expect(InboxWords.breadcrumb(for: message()) == "Gentbox › Phase 3")
		let general = GroupContext(
			topicTitle: "General", topicIsGeneral: true, topicIsMuted: false,
			threadTitle: "Refactor the store", threadIsMuted: false,
		)
		#expect(InboxWords.breadcrumb(for: message(group: general)) == "Refactor the store")
		let sameName = GroupContext(
			topicTitle: "Web", topicIsGeneral: false, topicIsMuted: false, threadTitle: "web", threadIsMuted: false,
		)
		#expect(InboxWords.breadcrumb(for: message(group: sameName)) == "Web")
		#expect(InboxWords.breadcrumb(for: message(group: nil, conversationTitle: "ingest")) == "ingest")
	}

	@Test func mutedByTopicOrThread() {
		#expect(!InboxWords.isMuted(message()))
		let thread = GroupContext(
			topicTitle: "Web", topicIsGeneral: false, topicIsMuted: false, threadTitle: "Search", threadIsMuted: true,
		)
		#expect(InboxWords.isMuted(message(group: thread)))
	}

	@Test func knownAgentsGetTheirMarks() {
		#expect(InboxWords.agentMark(for: "claude-code") == .claude)
		#expect(InboxWords.agentMark(for: "Claude") == .claude)
		#expect(InboxWords.agentMark(for: "codex") == .codex)
		#expect(InboxWords.agentMark(for: "aider") == .other)
	}

	@Test func permissionPromptsGetTheShield() {
		// The shield stays when the waiter lapses and Approve is hidden.
		let prompt = message(type: .decisionRequired, asksPermission: true)
		let question = message(type: .decisionRequired, actions: [action("a", .choice)])
		#expect(InboxWords.typeSymbol(for: prompt) == "checkmark.shield")
		#expect(InboxWords.typeSymbol(for: question) == "arrow.triangle.branch")
		#expect(InboxWords.typeSymbol(for: message(type: .actionRequired)) == "hand.tap")
		#expect(InboxWords.typeSymbol(for: message(type: .generic)) == "info.circle")
	}

	@Test func mutedSummary() {
		#expect(InboxWords.mutedSummary([]) == nil)
		let deps = MutedGroup(kind: .topic, id: "t", title: "Dependency upgrades")
		#expect(InboxWords.mutedSummary([deps]) == "Dependency upgrades muted")
		#expect(InboxWords.mutedSummary([deps, deps]) == "2 topics and threads muted")
	}

	@Test func copyHasNoTrailingPeriods() {
		let lines = [
			InboxWords.mutedSummary([MutedGroup(kind: .thread, id: "t", title: "v1.1")]),
			InboxWords.waitingLabel(for: .blocking),
			InboxWords.waitingLabel(for: .deferrable),
		]
		#expect(lines.compactMap { $0 }.allSatisfy { !$0.hasSuffix(".") })
	}
}

@Suite("SF Symbols")
struct SymbolPolicyTests {
	@Test func everySymbolInUseExistsAndIsUnrestricted() throws {
		let restricted = try #require(SymbolPolicy.systemRestrictions())
		for name in InboxWords.symbolsInUse {
			#expect(!restricted.contains(name), "\(name) is restricted")
			#expect(NSImage(systemSymbolName: name, accessibilityDescription: nil) != nil, "\(name) is missing")
		}
	}

	@Test func agentSymbolsFallBackWhenRestrictedOrMissing() {
		let policy = SymbolPolicy(restricted: ["iphone"]) { $0 != "no.such.symbol" }
		#expect(policy.symbol("shippingbox", fallback: "folder") == "shippingbox")
		#expect(policy.symbol("iphone", fallback: "folder") == "folder")
		#expect(policy.symbol("no.such.symbol", fallback: "folder") == "folder")
		#expect(policy.symbol(nil, fallback: "folder") == "folder")
		let unreadable = SymbolPolicy(restricted: nil) { _ in true }
		#expect(unreadable.symbol("shippingbox", fallback: "folder") == "folder")
	}
}
