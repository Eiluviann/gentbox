import GentboxCore
import SwiftUI

/// The type symbol, with the red urgent badge under it in the same column.
/// Urgency is the badge and the sort, never a label or a title prefix.
struct TypeIcon: View {
	let message: InboxMessage
	var size: CGFloat = 14

	var body: some View {
		VStack(spacing: 3) {
			Image(systemName: InboxWords.typeSymbol(for: message))
				.font(.system(size: size))
				.foregroundStyle(Self.color(for: message.messageType))
			if message.isUrgent {
				Image(systemName: "figure.walk.motion")
					.font(.system(size: size - 2))
					.foregroundStyle(Tokens.waiting)
					.accessibilityLabel("Urgent")
			}
		}
		.frame(width: Tokens.iconColumn)
		// The symbol sits on the first text line.
		.padding(.top, 1)
	}

	static func color(for type: MessageType) -> Color {
		switch type {
		case .decisionRequired: Tokens.decision
		case .actionRequired: Tokens.action
		case .taskCompleted: Tokens.done
		case .generic: Tokens.info
		}
	}
}

/// `pause.circle` on the right: red while the agent waits, orange while it
/// is paused and can be deferred; nothing while it keeps working.
struct WaitingMark: View {
	let blocking: Blocking

	var body: some View {
		if let label = InboxWords.waitingLabel(for: blocking) {
			Image(systemName: "pause.circle")
				.font(.system(size: 14))
				.foregroundStyle(blocking == .blocking ? Tokens.waiting : Tokens.paused)
				.accessibilityLabel(label)
		}
	}
}

/// A message's second line: conversation · agent · time ago.
struct RowContext: View {
	let message: InboxMessage

	var body: some View {
		TimelineView(.periodic(from: .now, by: 60)) { timeline in
			Text(InboxWords.rowContext(for: message, now: timeline.date).joined(separator: " · "))
				.font(.system(size: 11))
				.foregroundStyle(.secondary)
				.lineLimit(1)
		}
	}
}

/// A two-line row: type icon, title, context, and the waiting mark. Its
/// menu (right-click or the ellipsis on hover) snoozes or dismisses it and
/// mutes, closes or moves its thread and topic.
struct MessageRow: View {
	let message: InboxMessage
	let thread: InboxThread
	let topic: InboxTopic
	let moveTargets: [TopicChoice]
	let inbox: InboxModel
	@State private var isHovered = false

	var body: some View {
		HStack(alignment: .top, spacing: Tokens.columnGap) {
			TypeIcon(message: message)
			VStack(alignment: .leading, spacing: 1) {
				Text(message.title)
					.font(.system(size: 13, weight: message.state == .open ? .semibold : .medium))
					.lineLimit(1)
					.truncationMode(.tail)
				RowContext(message: message)
			}
			.frame(maxWidth: .infinity, alignment: .leading)
			ZStack {
				WaitingMark(blocking: message.blocking)
					.opacity(isHovered ? 0 : 1)
				Menu {
					menuItems
				} label: {
					Image(systemName: "ellipsis")
						.foregroundStyle(.secondary)
				}
				.menuStyle(.borderlessButton)
				.menuIndicator(.hidden)
				.fixedSize()
				.opacity(isHovered ? 1 : 0)
				.accessibilityLabel("More")
				.accessibilityIdentifier("message-menu:\(message.title)")
			}
			.frame(width: 22)
		}
		.padding(.horizontal, Tokens.rowHorizontalPadding)
		.padding(.vertical, 9)
		.background(isHovered ? Tokens.track : .clear)
		.contentShape(Rectangle())
		.onHover { isHovered = $0 }
		.contextMenu { menuItems }
		.accessibilityElement(children: .contain)
		.accessibilityIdentifier("message:\(message.title)")
	}

	@ViewBuilder private var menuItems: some View {
		Button {
			Task { await inbox.snooze(message) }
		} label: {
			Label("Snooze 15 min", systemImage: "moon.zzz")
		}
		if message.messageType != .actionRequired {
			Button {
				Task { await inbox.dismiss(message) }
			} label: {
				Label("Dismiss", systemImage: "xmark")
			}
		}
		Divider()
		ThreadMenu(thread: thread, topic: topic, moveTargets: moveTargets, inbox: inbox)
		Divider()
		TopicMenu(topic: topic, inbox: inbox)
	}
}

/// The message an agent is blocked on: the full card with its body and the
/// buttons it offers now (design: "Next up").
struct NextUpCard: View {
	let message: InboxMessage
	let inbox: InboxModel
	@State private var reply = ""
	@State private var isReplying = false

	var body: some View {
		VStack(alignment: .leading, spacing: 0) {
			HStack(alignment: .top, spacing: Tokens.columnGap) {
				TypeIcon(message: message, size: 15)
				VStack(alignment: .leading, spacing: 2) {
					Text(message.title)
						.font(.system(size: 15, weight: .semibold))
						.fixedSize(horizontal: false, vertical: true)
					RowContext(message: message)
				}
				.frame(maxWidth: .infinity, alignment: .leading)
				WaitingMark(blocking: message.blocking)
			}
			VStack(alignment: .leading, spacing: 0) {
				if let summary = message.summary {
					Text(summary)
						.font(.system(size: 13))
						.foregroundStyle(.secondary)
						.padding(.top, 6)
				}
				if let body = message.body {
					// The body carries commands, paths and raw input (ADR 0010);
					// the popover shows its start in a code box.
					Text(body)
						.font(.system(size: 12, design: .monospaced))
						.foregroundStyle(.secondary)
						.lineLimit(4)
						.textSelection(.enabled)
						.padding(.top, 6)
				}
				actionBar
					.padding(.top, 12)
			}
			.padding(.leading, Tokens.iconColumn + Tokens.columnGap)
		}
		.padding(EdgeInsets(top: 14, leading: Tokens.rowHorizontalPadding, bottom: 16, trailing: Tokens.rowHorizontalPadding))
		.accessibilityElement(children: .contain)
		.accessibilityIdentifier("next-up:\(message.title)")
	}

	@ViewBuilder private var actionBar: some View {
		let answers = message.offeredActions.filter { $0.kind != .snooze && $0.kind != .open }
		if isReplying, let replyAction = answers.first(where: { $0.kind == .reply }) {
			HStack(spacing: 6) {
				TextField(replyAction.placeholder ?? "Reply", text: $reply)
					.textFieldStyle(.roundedBorder)
					.onSubmit { send(replyAction) }
				Button("Send") { send(replyAction) }
					.keyboardShortcut(.defaultAction)
					.disabled(reply.trimmingCharacters(in: .whitespaces).isEmpty)
				Button("Cancel") { isReplying = false }
			}
		} else if answers.isEmpty {
			// No live waiter: answer buttons stay hidden (architecture §9.1).
			Button("Snooze 15 min") { Task { await inbox.snooze(message) } }
				.controlSize(.regular)
		} else {
			FlowButtons(actions: answers) { action in
				if action.kind == .reply {
					isReplying = true
				} else {
					Task { await inbox.press(action, on: message) }
				}
			}
		}
	}

	private func send(_ action: InboxAction) {
		let text = reply.trimmingCharacters(in: .whitespaces)
		guard !text.isEmpty else { return }
		isReplying = false
		reply = ""
		Task { await inbox.press(action, on: message, reply: text) }
	}
}

/// The card's buttons: the first approve or Done is the default, Deny and
/// destructive ones are red.
private struct FlowButtons: View {
	let actions: [InboxAction]
	let press: (InboxAction) -> Void

	var body: some View {
		HStack(spacing: 6) {
			ForEach(actions, id: \.id) { action in
				AnswerButton(action: action, isPrimary: action.id == primaryId) { press(action) }
			}
		}
	}

	private var primaryId: String? {
		actions.first { $0.kind == .approve || $0.kind == .done }?.id
	}
}

private struct AnswerButton: View {
	let action: InboxAction
	let isPrimary: Bool
	let press: () -> Void

	var body: some View {
		if isPrimary {
			button.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
		} else {
			button.buttonStyle(.bordered)
		}
	}

	private var button: some View {
		Button(action: press) {
			if isRed {
				Text(action.label).foregroundStyle(Color.red)
			} else {
				// The style picks the label colour: white on the accent while
				// the window is key, ink when it is not.
				Text(action.label)
			}
		}
		.accessibilityIdentifier("action:\(action.label)")
	}

	private var isRed: Bool {
		!isPrimary && (action.kind == .deny || action.isDestructive)
	}
}
