import AppKit
import GentboxCore
import SwiftUI

/// Agent-chosen topic symbols, drawn only when allowed (`SymbolPolicy`).
@MainActor
enum TopicSymbols {
	static let policy = SymbolPolicy { NSImage(systemSymbolName: $0, accessibilityDescription: nil) != nil }

	static func symbol(for topic: InboxTopic) -> String {
		policy.symbol(topic.symbol, fallback: topic.isGeneral ? "tray" : "folder")
	}
}

/// A topic's header row: its symbol, its title, and a bell when muted. Its
/// menu mutes, marks read and closes the topic.
struct TopicHeader: View {
	let topic: InboxTopic
	let inbox: InboxModel

	var body: some View {
		HStack(spacing: 9) {
			Image(systemName: TopicSymbols.symbol(for: topic))
				.font(.system(size: 14))
				.foregroundStyle(.secondary)
				.frame(width: 18)
			Text(topic.title)
				.font(.system(size: 13, weight: .semibold))
				.lineLimit(1)
				.frame(maxWidth: .infinity, alignment: .leading)
			if topic.isMuted {
				MutedMark()
			}
		}
		.padding(EdgeInsets(top: 12, leading: 13, bottom: 4, trailing: 12))
		.contentShape(Rectangle())
		.contextMenu { TopicMenu(topic: topic, inbox: inbox) }
		.accessibilityElement(children: .combine)
		.accessibilityAddTraits(.isHeader)
		.accessibilityValue(topic.isMuted ? "Muted" : "")
		.accessibilityIdentifier("topic:\(topic.title)")
	}
}

/// A thread's header row, indented to the rows' titles.
struct ThreadHeader: View {
	let thread: InboxThread
	let topic: InboxTopic
	let moveTargets: [TopicChoice]
	let inbox: InboxModel

	var body: some View {
		HStack(spacing: 6) {
			Text(thread.title)
				.font(.system(size: 12, weight: .medium))
				.foregroundStyle(.secondary)
				.lineLimit(1)
				.frame(maxWidth: .infinity, alignment: .leading)
			if thread.isMuted {
				MutedMark()
			}
		}
		.padding(EdgeInsets(top: 6, leading: Tokens.threadIndent, bottom: 2, trailing: 12))
		.contentShape(Rectangle())
		.contextMenu {
			ThreadMenu(thread: thread, topic: topic, moveTargets: moveTargets, inbox: inbox)
		}
		.accessibilityElement(children: .combine)
		.accessibilityAddTraits(.isHeader)
		.accessibilityValue(thread.isMuted ? "Muted" : "")
		.accessibilityIdentifier("thread:\(thread.title)")
	}
}

/// The bell on a muted header; the header says "Muted" to VoiceOver.
private struct MutedMark: View {
	var body: some View {
		Image(systemName: "bell.slash")
			.font(.system(size: 11))
			.foregroundStyle(.secondary)
			.accessibilityHidden(true)
	}
}

/// A topic's actions, in its header's menu and in each message's menu.
struct TopicMenu: View {
	let topic: InboxTopic
	let inbox: InboxModel

	var body: some View {
		Button {
			Task { await inbox.setTopicMuted(topic.id, !topic.isMuted) }
		} label: {
			Label(
				topic.isMuted ? "Unmute topic \(topic.title)" : "Mute topic \(topic.title)",
				systemImage: topic.isMuted ? "bell" : "bell.slash",
			)
		}
		Button {
			Task { await inbox.markTopicRead(topic.id) }
		} label: {
			Label("Mark topic read", systemImage: "envelope.open")
		}
		Button {
			Task { await inbox.closeTopic(topic.id, withdrawingOpenMessages: false) }
		} label: {
			Label("Close topic", systemImage: "archivebox")
		}
		Button {
			Task { await inbox.closeTopic(topic.id, withdrawingOpenMessages: true) }
		} label: {
			Label("Close topic and withdraw its messages", systemImage: "archivebox")
		}
	}
}

/// A thread's actions, in its header's menu and in each message's menu.
struct ThreadMenu: View {
	let thread: InboxThread
	let topic: InboxTopic
	let moveTargets: [TopicChoice]
	let inbox: InboxModel

	var body: some View {
		Button {
			Task { await inbox.setThreadMuted(thread.id, !thread.isMuted) }
		} label: {
			Label(
				thread.isMuted ? "Unmute thread \(thread.title)" : "Mute thread \(thread.title)",
				systemImage: thread.isMuted ? "bell" : "bell.slash",
			)
		}
		Button {
			Task { await inbox.markThreadRead(thread.id) }
		} label: {
			Label("Mark thread read", systemImage: "envelope.open")
		}
		Button {
			Task { await inbox.closeThread(thread.id, withdrawingOpenMessages: false) }
		} label: {
			Label("Close thread", systemImage: "archivebox")
		}
		Button {
			Task { await inbox.closeThread(thread.id, withdrawingOpenMessages: true) }
		} label: {
			Label("Close thread and withdraw its messages", systemImage: "archivebox")
		}
		let targets = moveTargets.filter { $0.id != topic.id }
		if !targets.isEmpty {
			Menu {
				ForEach(targets, id: \.id) { target in
					Button(target.title) {
						Task { await inbox.moveThread(thread.id, toTopic: target.id) }
					}
				}
			} label: {
				Label("Move to topic", systemImage: "folder")
			}
		}
	}
}

/// The Closed section: closed topics and threads, each with Reopen.
struct ClosedSection: View {
	let groups: [ClosedGroup]
	let inbox: InboxModel
	@State private var isExpanded = false

	var body: some View {
		Divider()
			.padding(.top, 6)
		DisclosureGroup(isExpanded: $isExpanded) {
			VStack(alignment: .leading, spacing: 0) {
				ForEach(groups, id: \.id) { group in
					ClosedRow(group: group, inbox: inbox)
				}
			}
		} label: {
			Text("Closed")
				.font(.system(size: 12, weight: .medium))
				.foregroundStyle(.secondary)
				.frame(maxWidth: .infinity, alignment: .leading)
				.contentShape(Rectangle())
				.onTapGesture { isExpanded.toggle() }
		}
		.padding(EdgeInsets(top: 8, leading: 14, bottom: 8, trailing: 12))
		.accessibilityIdentifier("inbox.closed")
	}
}

private struct ClosedRow: View {
	let group: ClosedGroup
	let inbox: InboxModel

	var body: some View {
		HStack(spacing: 8) {
			Image(systemName: "archivebox")
				.font(.system(size: 12))
				.foregroundStyle(.tertiary)
				.frame(width: Tokens.iconColumn)
			VStack(alignment: .leading, spacing: 0) {
				Text(group.title)
					.font(.system(size: 12))
					.lineLimit(1)
				if let topicTitle = group.topicTitle {
					Text(topicTitle)
						.font(.caption)
						.foregroundStyle(.secondary)
						.lineLimit(1)
				}
			}
			.frame(maxWidth: .infinity, alignment: .leading)
			Button("Reopen") {
				Task { await inbox.reopen(group) }
			}
			.controlSize(.small)
			.accessibilityIdentifier("reopen:\(group.title)")
		}
		.padding(.vertical, 4)
		.accessibilityIdentifier("closed:\(group.title)")
	}
}
