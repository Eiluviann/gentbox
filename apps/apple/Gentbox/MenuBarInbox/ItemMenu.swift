import GentboxCore
import SwiftUI

/// A message's menu (design: `mac-popover-item-menu`), on hover or
/// right-click: open the conversation, snooze, and mute, close or move its
/// thread and topic. Topic and thread rows are gone from the popover, so
/// their actions live here.
struct ItemMenu: View {
	let message: InboxMessage
	let moveTargets: [TopicChoice]
	let inbox: InboxModel

	var body: some View {
		if let open = message.offeredActions.first(where: { $0.kind == .open }) {
			Button {
				Task { await inbox.press(open, on: message) }
			} label: {
				Label("Open conversation", systemImage: "arrow.up.forward.app")
			}
		}
		Button {
			Task { await inbox.snooze(message) }
		} label: {
			Label("Snooze 15 min", systemImage: "moon.zzz")
		}
		if let group = message.group {
			Divider()
			Button {
				Task { await inbox.setThreadMuted(message.threadId, !group.threadIsMuted) }
			} label: {
				Label(
					group.threadIsMuted ? "Unmute thread \(group.threadTitle)" : "Mute thread \(group.threadTitle)",
					systemImage: group.threadIsMuted ? "bell" : "bell.slash",
				)
			}
			Button {
				Task { await inbox.setTopicMuted(message.topicId, !group.topicIsMuted) }
			} label: {
				Label(
					group.topicIsMuted ? "Unmute topic \(group.topicTitle)" : "Mute topic \(group.topicTitle)",
					systemImage: group.topicIsMuted ? "bell" : "bell.slash",
				)
			}
			// A thread closes only when nothing in it is open; while items are,
			// Close shows them instead ("N items still open").
			Button {
				Task { _ = await inbox.closeThread(of: message) }
			} label: {
				Label("Close thread", systemImage: "archivebox")
			}
			let targets = moveTargets.filter { $0.id != message.topicId }
			if !targets.isEmpty {
				Menu {
					ForEach(targets, id: \.id) { target in
						Button(target.title) {
							Task { await inbox.moveThread(message.threadId, toTopic: target.id) }
						}
					}
				} label: {
					Label("Move to topic", systemImage: "folder")
				}
			}
		}
	}
}
