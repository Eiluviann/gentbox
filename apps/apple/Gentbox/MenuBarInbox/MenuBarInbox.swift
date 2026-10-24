import GentboxCore
import SwiftUI

/// The menu-bar inbox (design: `mac-popover`): the message an agent is
/// blocked on as a full card, then the open messages grouped topic → thread
/// → messages, the Closed section, and a footer naming what is muted. No
/// counters, no kind labels.
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

/// "Inbox", and the menu with Open at Login and Quit.
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
			.accessibilityLabel("Settings")
			.accessibilityIdentifier("inbox.settings")
		}
		.padding(EdgeInsets(top: 12, leading: 14, bottom: 10, trailing: 12))
		.onAppear { app.loginItem.refresh() }
	}
}

/// The running inbox.
private struct InboxContent: View {
	let inbox: InboxModel

	var body: some View {
		if let snapshot = inbox.inbox {
			if inbox.isEmpty, snapshot.closed.isEmpty {
				InboxNotice(symbol: "tray", title: "No agent needs you", caption: nil)
			} else {
				ScrollView {
					LazyVStack(alignment: .leading, spacing: 0) {
						if let next = snapshot.nextUp {
							NextUpCard(message: next, inbox: inbox)
							Divider()
						}
						ForEach(snapshot.topics, id: \.id) { topic in
							TopicSection(topic: topic, snapshot: snapshot, inbox: inbox)
						}
						if !snapshot.closed.isEmpty {
							ClosedSection(groups: snapshot.closed, inbox: inbox)
						}
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

/// A topic header, then each thread's header and rows.
private struct TopicSection: View {
	let topic: InboxTopic
	let snapshot: Inbox
	let inbox: InboxModel

	var body: some View {
		TopicHeader(topic: topic, inbox: inbox)
		ForEach(topic.threads, id: \.id) { thread in
			ThreadHeader(thread: thread, topic: topic, moveTargets: snapshot.moveTargets, inbox: inbox)
			ForEach(thread.messages, id: \.id) { message in
				MessageRow(message: message, thread: thread, topic: topic, moveTargets: snapshot.moveTargets, inbox: inbox)
			}
		}
	}
}

/// The foot: what is muted (with Unmute), and the last failure.
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
						.font(.caption)
						.foregroundStyle(Tokens.waiting)
						.accessibilityIdentifier("inbox.failure")
				}
				if let muted {
					HStack(spacing: 8) {
						Image(systemName: "bell.slash")
							.font(.caption)
							.foregroundStyle(.secondary)
						Text(muted)
							.font(.caption)
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
						.font(.caption)
						.accessibilityIdentifier("inbox.unmute")
					}
				}
			}
			.padding(EdgeInsets(top: 9, leading: 14, bottom: 9, trailing: 12))
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
					.font(.caption)
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
