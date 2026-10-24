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

/// The agent's mark: Claude's and Codex's brand marks, a terminal for any
/// other agent. VoiceOver reads the agent's name.
struct AgentMark: View {
	let message: InboxMessage
	var size: CGFloat = 11

	var body: some View {
		image
			.frame(width: size, height: size)
			.accessibilityLabel(message.agentName)
	}

	@ViewBuilder private var image: some View {
		switch InboxWords.agentMark(for: message.agent) {
		case .claude:
			Image("AgentClaude").resizable().scaledToFit().foregroundStyle(Tokens.claude)
		case .codex:
			Image("AgentCodex").resizable().scaledToFit().foregroundStyle(.secondary)
		case .other:
			Image(systemName: "terminal").resizable().scaledToFit().foregroundStyle(.secondary)
		}
	}
}

/// A row's second line: "Topic › Thread · [agent] 4 min ago", with faded dots.
struct RowContext: View {
	let message: InboxMessage

	var body: some View {
		TimelineView(.periodic(from: .now, by: 60)) { timeline in
			HStack(spacing: 4) {
				if let breadcrumb = InboxWords.breadcrumb(for: message) {
					Text(breadcrumb)
						.lineLimit(1)
						.truncationMode(.tail)
						.layoutPriority(-1)
					Text("·")
						.font(.system(size: 14, weight: .bold))
						.foregroundStyle(Tokens.dot)
						.accessibilityHidden(true)
				}
				AgentMark(message: message)
				Text(InboxWords.timeAgo(message.receivedAt, now: timeline.date))
					.fixedSize()
			}
			.font(.system(size: 11))
			.foregroundStyle(.secondary)
			.accessibilityElement(children: .combine)
			.accessibilityIdentifier("context:\(message.title)")
		}
	}
}

/// One item of the accordion (design: `mac-popover`: "the active card keeps
/// a row's order and adds its body below, so a row can grow into it").
/// Collapsed, it is a two-line row: type icon, title, breadcrumb, and the
/// waiting mark, which the ellipsis menu replaces on hover. Expanded, the
/// same header grows (the title to 15 points, more padding) and the body and
/// buttons are revealed below it, inside the item's own animated, clipped
/// frame: one view throughout, so nothing cross-fades. Only the expanded item
/// has buttons, so ↩, ⌘⌫ and R act on it alone.
///
/// A click on a collapsed item expands it (and collapses the open one); a
/// click on the expanded item's header collapses it; ⌘-click picks a
/// collapsed item for a bulk Dismiss (never an `action_required` one).
/// The animation itself comes from the list, the one source for it.
struct InboxItem: View {
	let message: InboxMessage
	let isExpanded: Bool
	let moveTargets: [TopicChoice]
	let inbox: InboxModel
	@State private var isHovered = false
	@State private var reply = ""
	@State private var isReplying = false

	var body: some View {
		VStack(alignment: .leading, spacing: 0) {
			header
			if isExpanded {
				details
					.padding(.leading, Tokens.iconColumn + Tokens.columnGap)
					.transition(Self.detailsTransition)
			}
		}
		.padding(.horizontal, Tokens.rowHorizontalPadding)
		.padding(.top, isExpanded ? 14 : 9)
		.padding(.bottom, isExpanded ? 16 : 9)
		.background(background)
		.clipped()
		.contentShape(Rectangle())
		.onHover { isHovered = $0 }
		.contextMenu {
			ItemMenu(message: message, moveTargets: moveTargets, inbox: inbox)
		}
		.accessibilityElement(children: .contain)
		.accessibilityAddTraits(isPicked ? [.isButton, .isSelected] : .isButton)
		.accessibilityValue(InboxWords.isMuted(message) ? "Muted" : "")
		.accessibilityIdentifier(isExpanded ? "card:\(message.title)" : "message:\(message.title)")
		.onChange(of: isExpanded) {
			isReplying = false
			reply = ""
		}
	}

	private var header: some View {
		HStack(alignment: .top, spacing: Tokens.columnGap) {
			TypeIcon(message: message)
			VStack(alignment: .leading, spacing: isExpanded ? 2 : 1) {
				Text(message.title)
					// The same style in both states: any change to the title's font
					// (a size or weight, animated or morphed) gives the text a
					// transition of its own, and it drifts from its breadcrumb
					// while the item springs. The card stands out by its body
					// and padding instead of the board's 15-point headline.
					.font(.system(size: 13, weight: .semibold))
					// One line in both states, as titles are written to fit
					// (ADR 0010): a title that rewrapped as it grew would
					// cross-fade between layouts and shift the breadcrumb.
					.lineLimit(1)
					.truncationMode(.tail)
				RowContext(message: message)
			}
			.frame(maxWidth: .infinity, alignment: .leading)
			ZStack {
				WaitingMark(blocking: message.blocking)
					.opacity(isHovered && !isExpanded ? 0 : 1)
				if !isExpanded {
					Menu {
						ItemMenu(message: message, moveTargets: moveTargets, inbox: inbox)
					} label: {
						Image(systemName: "ellipsis")
							.foregroundStyle(.secondary)
					}
					.menuStyle(.borderlessButton)
					.menuIndicator(.hidden)
					.fixedSize()
					.padding(3)
					.background(Tokens.track, in: RoundedRectangle(cornerRadius: 6))
					.opacity(isHovered ? 1 : 0)
					.accessibilityLabel("More")
					.accessibilityIdentifier("message-menu:\(message.title)")
				}
			}
			.frame(width: 22)
		}
		.contentShape(Rectangle())
		.onTapGesture {
			if !isExpanded, NSEvent.modifierFlags.contains(.command) {
				inbox.toggleSelection(of: message)
			} else {
				inbox.toggleExpansion(of: message.id)
			}
		}
	}

	private var details: some View {
		VStack(alignment: .leading, spacing: 0) {
			if let summary = message.summary {
				Text(summary)
					.font(.system(size: 13))
					.foregroundStyle(.secondary)
					.padding(.top, 6)
			}
			if let body = message.body {
				// Commands, paths and raw input live only in the body
				// (ADR 0010); the card shows its start in `.pp-cmd`.
				Text(body)
					.font(.system(size: 12, design: .monospaced))
					.lineLimit(4)
					.textSelection(.enabled)
					.frame(maxWidth: .infinity, alignment: .leading)
					.padding(.horizontal, 9)
					.padding(.vertical, 6)
					.background(Tokens.codeBox, in: RoundedRectangle(cornerRadius: 6))
					.padding(.top, 8)
			}
			actionBar
				.padding(.top, 12)
		}
	}

	/// An inserted view appears at its final place while its neighbours are
	/// still moving, so the body fades in only once the spring has mostly
	/// settled, and fades out at once on collapse, before the rows slide up:
	/// it never draws over a moving header.
	private static let detailsTransition = AnyTransition.asymmetric(
		insertion: .opacity.animation(.easeOut(duration: 0.15).delay(0.3)),
		removal: .opacity.animation(.easeOut(duration: 0.08)),
	)

	private var isPicked: Bool {
		inbox.selection.contains(message.id)
	}

	private var background: some View {
		(isExpanded ? Tokens.card : Tokens.row)
			.overlay(isPicked ? Tokens.picked : (isHovered && !isExpanded ? Tokens.hover : .clear))
	}

	@ViewBuilder private var actionBar: some View {
		let answers = message.offeredActions.filter { $0.kind != .snooze && $0.kind != .open }
		if isReplying, let replyAction = answers.first(where: { $0.kind == .reply }) {
			HStack(spacing: 6) {
				TextField(replyAction.placeholder ?? "Reply", text: $reply)
					.textFieldStyle(.roundedBorder)
					.onSubmit { send(replyAction) }
				Button("Send") { send(replyAction) }
					.buttonStyle(DesignButtonStyle(kind: .primary, hint: "↩"))
					.keyboardShortcut(.defaultAction)
					.disabled(reply.trimmingCharacters(in: .whitespaces).isEmpty)
				Button("Cancel") { isReplying = false }
					.buttonStyle(DesignButtonStyle())
					.keyboardShortcut(.cancelAction)
			}
		} else if !answers.isEmpty {
			AnswerButtons(actions: answers) { action in
				if action.kind == .reply {
					isReplying = true
				} else {
					Task { await inbox.press(action, on: message) }
				}
			}
		} else if message.messageType == .taskCompleted || message.messageType == .generic {
			Button("Dismiss") { Task { await inbox.dismiss(message) } }
				.buttonStyle(DesignButtonStyle())
				.accessibilityIdentifier("action:Dismiss")
		} else {
			// No live waiter: answers stay hidden (architecture §9.1).
			Button("Snooze 15 min") { Task { await inbox.snooze(message) } }
				.buttonStyle(DesignButtonStyle())
				.accessibilityIdentifier("action:Snooze")
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

/// The card's buttons, as the design draws them: Approve (or Done) is the
/// primary button with ↩, Deny is red with ⌘⌫, Reply carries R.
private struct AnswerButtons: View {
	let actions: [InboxAction]
	let press: (InboxAction) -> Void

	var body: some View {
		HStack(spacing: 6) {
			ForEach(actions, id: \.id) { action in
				answerButton(action)
			}
		}
	}

	@ViewBuilder private func answerButton(_ action: InboxAction) -> some View {
		let button = Button(action.label) { press(action) }
			.accessibilityIdentifier("action:\(action.label)")
		if action.id == primaryId {
			button
				.buttonStyle(DesignButtonStyle(kind: .primary, hint: "↩"))
				.keyboardShortcut(.defaultAction)
		} else if action.id == denyId {
			button
				.buttonStyle(DesignButtonStyle(kind: .danger, hint: "⌘⌫"))
				.keyboardShortcut(.delete, modifiers: .command)
		} else if action.id == replyId {
			button
				.buttonStyle(DesignButtonStyle(hint: "R"))
				.keyboardShortcut("r", modifiers: [])
		} else {
			button.buttonStyle(DesignButtonStyle(kind: action.isDestructive ? .danger : .normal))
		}
	}

	private var primaryId: String? {
		actions.first { $0.kind == .approve || $0.kind == .done }?.id
	}

	private var denyId: String? {
		actions.first { $0.kind == .deny }?.id
	}

	private var replyId: String? {
		actions.first { $0.kind == .reply }?.id
	}
}
