import GentboxCore
import SwiftUI

/// A message's menu (design: `mac-popover-item-menu`), on hover or
/// right-click: open the conversation, snooze, and mute, close or move its
/// thread and topic. Topic and thread rows are gone from the popover, so
/// their actions live here.
struct ItemMenu: View {
	/// The thread Close asks about.
	struct Closing: Equatable {
		let threadId: String
		let title: String
	}

	let message: InboxMessage
	let moveTargets: [TopicChoice]
	let inbox: InboxModel
	@Binding var closing: Closing?

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
			Button {
				closing = Closing(threadId: message.threadId, title: group.threadTitle)
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

extension View {
	/// Close asks whether to dismiss the thread's open messages too; a
	/// closed thread leaves the popover.
	func closeConfirmation(_ closing: Binding<ItemMenu.Closing?>, inbox: InboxModel) -> some View {
		alert(
			"Close \(closing.wrappedValue?.title ?? "thread")?",
			isPresented: Binding(get: { closing.wrappedValue != nil }, set: { if !$0 { closing.wrappedValue = nil } }),
			presenting: closing.wrappedValue,
		) { thread in
			Button("Close") {
				Task { await inbox.closeThread(thread.threadId, dismissingOpenMessages: false) }
			}
			Button("Close and dismiss its messages", role: .destructive) {
				Task { await inbox.closeThread(thread.threadId, dismissingOpenMessages: true) }
			}
			Button("Cancel", role: .cancel) {}
		} message: { _ in
			Text("Agents still waiting on its open messages keep waiting unless you dismiss them")
		}
	}
}
