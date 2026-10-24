import GentboxCore
import SwiftUI

/// The menu-bar inbox (design: `mac-popover`): the message an agent is
/// blocked on as the full card on white, everything else as a two-line list
/// on light grey with a rule between items, urgent first, then waiting. Topic
/// and thread show only in each row's breadcrumb; no section titles, no
/// counters. The popover lists only open, unhandled items.
struct MenuBarInbox: View {
	let app: AppModel

	var body: some View {
		VStack(spacing: 0) {
			InboxHeader(app: app)
			Divider()
			switch app.phase {
			case .starting:
				InboxNotice(symbol: "bubble.left", title: "Starting Gentbox", caption: nil)
			case .failed(let reason):
				InboxNotice(symbol: "xmark", title: "Gentbox could not start", caption: reason)
			case .running(let inbox):
				InboxContent(inbox: inbox)
			}
		}
		.frame(width: Tokens.popoverWidth)
	}
}

/// "Inbox", and the settings menu (Open at Login, Quit).
private struct InboxHeader: View {
	let app: AppModel

	var body: some View {
		HStack(spacing: 8) {
			Text("Inbox")
				.font(.system(size: 17, weight: .semibold))
				.frame(maxWidth: .infinity, alignment: .leading)
			Menu {
				Toggle(
					"Open at Login",
					isOn: Binding(
						get: { app.loginItem.isEnabled },
						set: { app.loginItem.setEnabled($0) },
					),
				)
				if app.loginItem.needsApproval {
					Text("Allow Gentbox in System Settings › Login Items")
				}
				Divider()
				Button("Quit Gentbox") { NSApp.terminate(nil) }
					.keyboardShortcut("q")
			} label: {
				Image(systemName: "gearshape")
					.foregroundStyle(.secondary)
			}
			.menuStyle(.borderlessButton)
			.menuIndicator(.hidden)
			.fixedSize()
			.frame(width: 28, height: 28)
			.accessibilityLabel("Settings")
			.accessibilityIdentifier("inbox.settings")
		}
		.padding(EdgeInsets(top: 12, leading: 14, bottom: 10, trailing: 12))
		.onAppear { app.loginItem.refresh() }
	}
}

/// The card, the list and the foot.
private struct InboxContent: View {
	let inbox: InboxModel

	var body: some View {
		if let snapshot = inbox.inbox {
			if inbox.isEmpty {
				InboxNotice(symbol: "tray", title: "No agent needs you", caption: nil)
			} else {
				ScrollView {
					VStack(alignment: .leading, spacing: 0) {
						if let card = inbox.card {
							MessageCard(message: card, inbox: inbox)
							Divider()
						}
						LazyVStack(alignment: .leading, spacing: 0) {
							ForEach(Array(inbox.list.enumerated()), id: \.element.id) { index, message in
								if index > 0 {
									Divider().padding(.leading, Tokens.rowHorizontalPadding)
								}
								MessageRow(message: message, moveTargets: snapshot.moveTargets, inbox: inbox)
							}
						}
						.background(Tokens.listBackground)
					}
				}
				.frame(maxHeight: Tokens.listMaxHeight)
				.fixedSize(horizontal: false, vertical: true)
			}
			InboxFooter(snapshot: snapshot, inbox: inbox)
		} else {
			InboxNotice(symbol: "tray", title: "Reading the inbox", caption: nil)
		}
	}
}

/// The foot (design: `mac-popover-item-menu`): what is muted, with Unmute,
/// and the last failure.
private struct InboxFooter: View {
	let snapshot: Inbox
	let inbox: InboxModel

	var body: some View {
		let muted = InboxWords.mutedSummary(snapshot.muted)
		if muted != nil || inbox.failure != nil {
			Divider()
			VStack(alignment: .leading, spacing: 6) {
				if let failure = inbox.failure {
					Label(failure, systemImage: "xmark")
						.font(.system(size: 11))
						.foregroundStyle(Tokens.waiting)
						.accessibilityIdentifier("inbox.failure")
				}
				if let muted {
					HStack(spacing: 8) {
						Image(systemName: "bell.slash")
							.font(.system(size: 12))
							.foregroundStyle(.secondary)
						Text(muted)
							.font(.system(size: 11))
							.foregroundStyle(.secondary)
							.frame(maxWidth: .infinity, alignment: .leading)
							.accessibilityIdentifier("inbox.muted-summary")
						Menu("Unmute") {
							ForEach(snapshot.muted, id: \.id) { group in
								Button(group.title) {
									Task {
										switch group.kind {
										case .topic: await inbox.setTopicMuted(group.id, false)
										case .thread: await inbox.setThreadMuted(group.id, false)
										}
									}
								}
							}
						}
						.menuStyle(.borderlessButton)
						.menuIndicator(.hidden)
						.fixedSize()
						.font(.system(size: 12))
						.foregroundStyle(Color.accentColor)
						.accessibilityIdentifier("inbox.unmute")
					}
				}
			}
			.padding(EdgeInsets(top: 9, leading: 14, bottom: 9, trailing: 12))
			.background(Tokens.listBackground)
		}
	}
}

/// A centred symbol and a line or two, for starting, failure and empty.
struct InboxNotice: View {
	let symbol: String
	let title: String
	let caption: String?

	var body: some View {
		VStack(spacing: 4) {
			Image(systemName: symbol)
				.font(.system(size: 28))
				.foregroundStyle(.tertiary)
			Text(title)
				.font(.system(size: 13, weight: .semibold))
				.padding(.top, 4)
			if let caption {
				Text(caption)
					.font(.system(size: 11))
					.foregroundStyle(.secondary)
					.multilineTextAlignment(.center)
			}
		}
		.frame(maxWidth: .infinity)
		.padding(.vertical, 36)
		.padding(.horizontal, 20)
		.accessibilityElement(children: .combine)
		.accessibilityIdentifier("inbox.notice")
	}
}
