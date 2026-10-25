import AppKit
import GentboxCore

/// The expanded card's body under its header: the summary, the start of the
/// body in a code box (commands, paths and raw input live only there, ADR
/// 0010), and the action bar. Its buttons carry the card's keys: ↩ for the
/// primary answer, ⌘⌫ for Deny, R for the reply field, where ↩ sends and ⎋
/// cancels.
///
/// A row builds its body ahead of being expanded (`IdleWork`), and keeps it
/// while collapsed, transparent and clipped away: expanding only reveals it.
/// Only an active body (its row expanded) answers clicks, keys and
/// VoiceOver.
@MainActor
final class CardBodyView: FlippedView, NSTextFieldDelegate {
	private let inbox: InboxModel
	private let summary = Label.wrapping(font: .systemFont(ofSize: 13), color: .secondaryLabelColor)
	private let codeBox = FlippedView()
	private let code = Label.wrapping(font: .monospacedSystemFont(ofSize: 12, weight: .regular), maxLines: 4)
	private(set) var message: InboxMessage?
	private var replyDraft: InboxModel.ReplyDraft?
	/// The action bar's controls for the message's actions, shown.
	private var controls: ActionControls?
	/// Controls for actions that changed (a waiter arrived), built ahead and
	/// swapped in once complete.
	private var nextControls: ActionControls?
	/// ↩ reaches both Send and the field: one answer per reply.
	private var hasSent = false

	/// Its row is expanded and staying: it takes clicks, keys and VoiceOver.
	var isActive = false {
		didSet {
			guard isActive != oldValue else { return }
			if isActive {
				keys.cardShown(at: CACurrentMediaTime())
			}
			setAccessibilityElement(isActive)
			showBar()
		}
	}

	private var keys = CardKeyGate()

	init(inbox: InboxModel) {
		self.inbox = inbox
		super.init(frame: .zero)
		codeBox.fill = Tokens.codeBox
		codeBox.cornerRadius = 6
		code.isSelectable = true
		codeBox.addSubview(code)
		addSubview(summary)
		addSubview(codeBox)
		setAccessibilityElement(false)
		setAccessibilityRole(.group)
	}

	func show(message: InboxMessage, replyDraft: InboxModel.ReplyDraft?) {
		keys.show(messageId: message.id, isShown: isActive, at: CACurrentMediaTime())
		if message != self.message {
			self.message = message
			summary.stringValue = message.summary ?? ""
			summary.isHidden = message.summary == nil
			code.stringValue = message.body ?? ""
			codeBox.isHidden = message.body == nil
			prepareControls(for: ActionControls.Spec(message: message))
		}
		let wasReplying = isReplying
		self.replyDraft = replyDraft
		if isReplying != wasReplying {
			hasSent = false
			showBar()
		}
		syncReplyField()
	}

	/// Builds whatever controls are still missing now: the body is about to
	/// show.
	func finishControls() {
		if let nextControls {
			nextControls.buildAll(in: self)
			swapIn(nextControls)
		}
		controls?.buildAll(in: self)
		showBar()
	}

	/// Lays out its parts for `width` and returns its height.
	func layOut(width: CGFloat) -> CGFloat {
		var y: CGFloat = 0
		if !summary.isHidden {
			y += 6
			let height = Label.height(of: summary, width: width)
			summary.frame = Label.frame(x: 0, y: y, width: width, height: height)
			y += height
		}
		if !codeBox.isHidden {
			y += 8
			let textHeight = Label.height(of: code, width: width - 18)
			codeBox.frame = NSRect(x: 0, y: y, width: width, height: textHeight + 12)
			code.frame = Label.frame(x: 9, y: 6, width: width - 18, height: textHeight)
			y += textHeight + 12
		}
		y += 12
		for bar in [controls, nextControls].compactMap({ $0 }) {
			bar.layOut(at: y, width: width)
		}
		return y + (controls?.height(isReplying: isReplying, width: width) ?? ActionControls.barHeight)
	}

	/// Notes where the reply field and its buttons show now: the field grows
	/// a line at a time with the card's height.
	func capture(into motion: inout LayerMotion) {
		guard let controls else { return }
		motion.capture(controls.replyField?.layer, ["position", "bounds"])
		for view in [controls.send, controls.cancel] as [NSView?] {
			motion.capture(view?.layer, ["position"])
		}
	}

	// MARK: Controls

	private var isReplying: Bool {
		replyDraft != nil && controls?.spec.reply != nil
	}

	private func prepareControls(for spec: ActionControls.Spec) {
		guard spec != controls?.spec else {
			nextControls?.remove()
			nextControls = nil
			return
		}
		guard spec != nextControls?.spec else { return }
		nextControls?.remove()
		let next = ActionControls(spec: spec)
		if controls == nil {
			controls = next
		} else {
			nextControls = next
		}
		buildAhead(next)
	}

	/// One control per quiet turn; a replacement shows once it is whole.
	private func buildAhead(_ bar: ActionControls) {
		IdleWork.enqueue { [weak self, weak bar] in
			guard let self, let bar, bar === self.controls || bar === self.nextControls else { return }
			if bar.buildNext(in: self) {
				self.buildAhead(bar)
			} else if bar === self.nextControls {
				self.swapIn(bar)
			} else {
				_ = self.layOut(width: self.bounds.width)
				self.showBar()
			}
		}
	}

	private func swapIn(_ bar: ActionControls) {
		controls?.remove()
		controls = bar
		nextControls = nil
		_ = layOut(width: bounds.width)
		showBar()
		syncReplyField()
	}

	/// The views the bar shows now.
	private var shownViews: [NSView] {
		isActive ? controls?.views(isReplying: isReplying) ?? [] : []
	}

	/// The bar that applies shows; every other control stays drawn at zero
	/// opacity, out of reach of clicks, keys and VoiceOver.
	private func showBar() {
		let shown = shownViews
		for bar in [controls, nextControls].compactMap({ $0 }) {
			for view in bar.allViews {
				let isShown = shown.contains { $0 === view }
				view.alphaValue = isShown ? 1 : 0
				view.setInReach(isShown)
			}
		}
		if isActive, isReplying, let field = controls?.replyField {
			if field.currentEditor() == nil {
				window?.makeFirstResponder(field)
			}
		} else if let field = controls?.replyField, field.currentEditor() != nil {
			window?.makeFirstResponder(nil)
		}
	}

	/// The controls shown take clicks; the concealed ones never do, even
	/// where one lies over a shown one (the reply field over the answers).
	/// `FlippedView` answers a concealed part with the body itself, so a hit
	/// on the body is looked at again too.
	override func hitTest(_ point: NSPoint) -> NSView? {
		guard isActive, let hit = super.hitTest(point) else { return nil }
		let shown = shownViews
		if shown.contains(where: { hit === $0 || hit.isDescendant(of: $0) }) {
			return hit
		}
		let local = convert(point, from: superview)
		for view in shown.reversed() {
			if let shownHit = view.hitTest(local) {
				return shownHit
			}
		}
		let barViews = [controls, nextControls].compactMap { $0 }.flatMap(\.allViews)
		return barViews.contains { hit === $0 || hit.isDescendant(of: $0) } ? self : hit
	}

	override func performKeyEquivalent(with event: NSEvent) -> Bool {
		guard isActive, !event.isARepeat, keys.isOpen(at: CACurrentMediaTime()) else { return false }
		return shownViews.contains { $0.performKeyEquivalent(with: event) }
	}

	override func viewDidMoveToWindow() {
		super.viewDidMoveToWindow()
		showBar()
	}

	// MARK: Actions

	func press(_ action: InboxAction) {
		guard let message else { return }
		if action.kind == .reply {
			inbox.startReply(on: message.id)
		} else {
			Interactions.untilNextFrame("Action")
			if InboxModel.isAnswer(action) {
				keys.answered()
			}
			inbox.takeOutAhead(message, for: action)
			Task { await inbox.press(action, on: message) }
		}
	}

	func dismiss() {
		guard let message else { return }
		Interactions.untilNextFrame("Action")
		keys.answered()
		inbox.takeOutAhead(message)
		Task { await inbox.dismiss(message) }
	}

	func snooze() {
		guard let message else { return }
		Interactions.untilNextFrame("Action")
		keys.answered()
		inbox.takeOutAhead(message)
		Task { await inbox.snooze(message) }
	}

	func cancelReply() {
		inbox.endReply()
	}

	// MARK: The reply field

	/// Typing updates the model to what the field holds, so this only
	/// changes the field when the draft came from elsewhere.
	private func syncReplyField() {
		guard let field = controls?.replyField else { return }
		let text = replyDraft?.text ?? ""
		if field.stringValue != text {
			field.stringValue = text
			// The caret after the text, as if typed.
			let end = NSRange(location: (text as NSString).length, length: 0)
			field.currentEditor()?.selectedRange = end
			field.currentEditor()?.scrollRangeToVisible(end)
		}
		controls?.send?.isEnabled = !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
	}

	func controlTextDidChange(_: Notification) {
		guard let field = controls?.replyField else { return }
		inbox.updateReply(field.stringValue)
		controls?.send?.isEnabled = !field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
	}

	@objc func sendReply() {
		guard let message, let field = controls?.replyField, let action = controls?.spec.reply, !hasSent else { return }
		let text = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
		guard !text.isEmpty else { return }
		hasSent = true
		keys.answered()
		inbox.endReply()
		Interactions.untilNextFrame("Action")
		inbox.takeOutAhead(message, for: action)
		Task { await inbox.press(action, on: message, reply: text) }
	}
}

/// The action bar's controls for one set of offered actions: the answers,
/// or Dismiss / Snooze when there are none, and the reply field with Send
/// and Cancel when the message takes a reply. Their actions go through the
/// body, to the message it shows now, so a superseding message with the
/// same actions keeps them.
@MainActor
final class ActionControls {
	struct Spec: Equatable {
		enum Fallback: Equatable {
			case dismiss
			case snooze
		}

		let answers: [InboxAction]
		let fallback: Fallback
		let reply: InboxAction?

		init(message: InboxMessage) {
			answers = message.offeredActions.filter { $0.kind != .snooze && $0.kind != .open }
			// No live waiter: answers stay hidden (architecture §9.1).
			fallback = message.messageType == .taskCompleted || message.messageType == .generic ? .dismiss : .snooze
			reply = answers.first { $0.kind == .reply }
		}
	}

	/// Native large controls: the 28-point `.btn`.
	static let barHeight: CGFloat = 28

	let spec: Spec
	private(set) var answerButtons: [InboxButton] = []
	private(set) var fallbackButton: InboxButton?
	private(set) var replyField: ReplyField?
	private(set) var send: InboxButton?
	private(set) var cancel: InboxButton?
	private var factories: [(ActionControls, CardBodyView) -> NSView]

	var isComplete: Bool {
		factories.isEmpty
	}

	var allViews: [NSView] {
		(answerButtons as [NSView]) + [fallbackButton, replyField, send, cancel].compactMap { $0 }
	}

	init(spec: Spec) {
		self.spec = spec
		factories = Self.factories(for: spec)
	}

	/// The views the bar shows.
	func views(isReplying: Bool) -> [NSView] {
		if isReplying, let replyField, let send, let cancel {
			return [replyField, send, cancel]
		}
		if !spec.answers.isEmpty {
			return answerButtons
		}
		return [fallbackButton].compactMap { $0 }
	}

	/// Builds the next control into `body`, drawn but transparent; whether
	/// more are left.
	func buildNext(in body: CardBodyView) -> Bool {
		guard !factories.isEmpty else { return false }
		let view = factories.removeFirst()(self, body)
		view.alphaValue = 0
		view.setInReach(false)
		body.addSubview(view)
		return !factories.isEmpty
	}

	func buildAll(in body: CardBodyView) {
		while buildNext(in: body) {}
	}

	func remove() {
		allViews.forEach { $0.removeFromSuperview() }
	}

	/// Space between the reply field and its buttons.
	static let replyGap: CGFloat = 8

	/// The height the shown bar takes: one row of buttons, or the reply field
	/// over its Send and Cancel.
	func height(isReplying: Bool, width: CGFloat) -> CGFloat {
		guard isReplying, let replyField else { return Self.barHeight }
		return replyField.height(width: width) + Self.replyGap + Self.barHeight
	}

	/// Each bar left to right at `y`; the reply field spans the width, with
	/// its buttons on the row below.
	func layOut(at y: CGFloat, width: CGFloat) {
		var buttonsTop = y
		if let replyField {
			replyField.frame = NSRect(x: 0, y: y, width: width, height: replyField.height(width: width))
			buttonsTop = replyField.frame.maxY + Self.replyGap
		}
		let bars: [(views: [NSView], top: CGFloat)] = [
			(answerButtons, y), ([fallbackButton].compactMap { $0 }, y), ([send, cancel].compactMap { $0 }, buttonsTop),
		]
		for bar in bars {
			var x: CGFloat = 0
			for view in bar.views {
				let top = bar.top + ((Self.barHeight - view.frame.height) / 2).rounded()
				view.frame = NSRect(x: x, y: top, width: view.frame.width, height: view.frame.height)
				x += view.frame.width + 6
			}
		}
	}

	private static func factories(for spec: Spec) -> [(ActionControls, CardBodyView) -> NSView] {
		var factories: [(ActionControls, CardBodyView) -> NSView] = []
		let primaryId = spec.answers.first { $0.kind == .approve || $0.kind == .done }?.id
		let denyId = spec.answers.first { $0.kind == .deny }?.id
		let replyId = spec.reply?.id
		for action in spec.answers {
			factories.append { controls, body in
				let button = answerButton(action, primaryId: primaryId, denyId: denyId, replyId: replyId, body: body)
				controls.answerButtons.append(button)
				return button
			}
		}
		if spec.answers.isEmpty {
			factories.append { controls, body in
				let button: InboxButton
				switch spec.fallback {
				case .dismiss:
					button = InboxButton(title: "Dismiss", role: .secondary) { [weak body] in body?.dismiss() }
					button.setAccessibilityIdentifier("action:Dismiss")
				case .snooze:
					button = InboxButton(title: "Snooze 15 min", role: .secondary) { [weak body] in body?.snooze() }
					button.setAccessibilityIdentifier("action:Snooze")
				}
				controls.fallbackButton = button
				return button
			}
		}
		if let reply = spec.reply {
			factories.append { controls, body in
				let field = ReplyField()
				field.placeholderString = reply.placeholder ?? "Reply"
				field.delegate = body
				field.target = body
				field.action = #selector(CardBodyView.sendReply)
				field.cell?.sendsActionOnEndEditing = false
				field.setAccessibilityIdentifier("reply-field")
				controls.replyField = field
				// What the first focus and the first keystroke would otherwise
				// set up mid-frame: the window's field editor and the shared
				// spell checker, each once.
				IdleWork.enqueue { [weak body, weak field] in
					guard let field else { return }
					_ = body?.window?.fieldEditor(true, for: field)
				}
				IdleWork.enqueue { _ = NSSpellChecker.shared }
				return field
			}
			factories.append { controls, body in
				let send = InboxButton(title: "Send", hint: "↩", role: .primary) { [weak body] in body?.sendReply() }
				send.keyEquivalent = "\r"
				send.isEnabled = false
				controls.send = send
				return send
			}
			factories.append { controls, body in
				let cancel = InboxButton(title: "Cancel", role: .secondary) { [weak body] in body?.cancelReply() }
				cancel.keyEquivalent = "\u{1b}"
				controls.cancel = cancel
				return cancel
			}
		}
		return factories
	}

	/// Approve (or Done) is the prominent button with ↩, Deny is
	/// destructive with ⌘⌫, Reply carries R.
	private static func answerButton(
		_ action: InboxAction, primaryId: String?, denyId: String?, replyId: String?, body: CardBodyView,
	) -> InboxButton {
		let role: ButtonRole =
			if action.id == primaryId {
				.primary
			} else if action.id == denyId || action.isDestructive {
				.destructive
			} else {
				.secondary
			}
		let hint: String? =
			switch action.id {
			case primaryId: "↩"
			case denyId: "⌘⌫"
			case replyId: "R"
			default: nil
			}
		let button = InboxButton(title: action.label, hint: hint, role: role) { [weak body] in body?.press(action) }
		switch action.id {
		case primaryId:
			button.keyEquivalent = "\r"
		case denyId:
			button.keyEquivalent = "\u{7f}"
			button.keyEquivalentModifierMask = .command
		case replyId:
			button.keyEquivalent = "r"
			button.keyEquivalentModifierMask = []
		default:
			break
		}
		button.setAccessibilityIdentifier("action:\(action.label)")
		return button
	}
}
