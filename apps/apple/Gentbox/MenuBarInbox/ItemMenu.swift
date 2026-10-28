import AppKit
import GentboxCore

/// A message's menu (design: `mac-popover-item-menu`), on hover or
/// right-click: open the conversation, snooze, and mute, close or move its
/// thread and topic. Topic and thread rows are gone from the popover, so
/// their actions live here. Built when it opens, never ahead.
@MainActor
enum ItemMenu {
	static func make(for message: InboxMessage, moveTargets: [TopicChoice], inbox: InboxModel) -> NSMenu {
		let menu = NSMenu()
		if let open = message.offeredActions.first(where: { $0.kind == .open }) {
			menu.addItem(ClosureMenuItem("Open conversation", symbol: "arrow.up.forward.app") {
				Task { await inbox.press(open, on: message) }
			})
		}
		menu.addItem(ClosureMenuItem("Snooze 15 min", symbol: "moon.zzz") {
			inbox.takeOutAhead(message)
			Task { await inbox.snooze(message) }
		})
		guard let group = message.group else { return menu }
		menu.addItem(.separator())
		menu.addItem(ClosureMenuItem(
			group.threadIsMuted ? "Unmute thread \(group.threadTitle)" : "Mute thread \(group.threadTitle)",
			symbol: group.threadIsMuted ? "bell" : "bell.slash",
		) {
			Task { await inbox.setThreadMuted(message.threadId, !group.threadIsMuted) }
		})
		menu.addItem(ClosureMenuItem(
			group.topicIsMuted ? "Unmute topic \(group.topicTitle)" : "Mute topic \(group.topicTitle)",
			symbol: group.topicIsMuted ? "bell" : "bell.slash",
		) {
			Task { await inbox.setTopicMuted(message.topicId, !group.topicIsMuted) }
		})
		// A thread closes only when nothing in it is open; while items are,
		// Close shows them instead ("N items still open").
		menu.addItem(ClosureMenuItem("Close thread", symbol: "archivebox") {
			Task { _ = await inbox.closeThread(of: message) }
		})
		let targets = moveTargets.filter { $0.id != message.topicId }
		if !targets.isEmpty {
			let move = NSMenuItem(title: "Move to topic", action: nil, keyEquivalent: "")
			move.image = NSImage(systemSymbolName: "folder", accessibilityDescription: nil)
			let submenu = NSMenu()
			for target in targets {
				submenu.addItem(ClosureMenuItem(target.title, symbol: nil) {
					Task { await inbox.moveThread(message.threadId, toTopic: target.id) }
				})
			}
			move.submenu = submenu
			menu.addItem(move)
		}
		return menu
	}
}

/// A menu item that runs a closure.
@MainActor
final class ClosureMenuItem: NSMenuItem {
	private let run: () -> Void

	init(_ title: String, symbol: String?, keyEquivalent: String = "", run: @escaping () -> Void) {
		self.run = run
		super.init(title: title, action: #selector(runClosure), keyEquivalent: keyEquivalent)
		target = self
		if let symbol {
			image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
		}
	}

	@available(*, unavailable)
	required init(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	@objc private func runClosure() {
		run()
	}
}
