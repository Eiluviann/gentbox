import AppKit
import GentboxCore

/// The expanded card's body under its header: the summary, the start of the
/// body in a code box (commands, paths and raw input live only there, ADR
/// 0010), and the action bar. Its buttons carry the card's keys: ↩ for the
/// primary answer, ⌘⌫ for Deny, R for the reply field, where ↩ sends and ⎋
/// cancels.
///
/// A stack of its own (ADR 0013): the summary, the code box and the bar's
/// rows are its parts, moved on the list's clock, so a reply box that opens
/// or takes a line makes room as the card does. A new message's text
/// cross-fades in place; a bar that replaces another in a part that stays
/// fades in growing slightly as the old one fades out shrinking.
///
/// A row builds its body ahead of being expanded (`IdleWork`), and keeps it
/// while collapsed, transparent and clipped away: expanding only reveals it.
/// Only an active body (its row expanded) answers clicks, keys and
/// VoiceOver.
@MainActor
final class CardBodyView: FlippedView, NSTextFieldDelegate, MotionStack {
	private enum Part: Hashable {
		case summary
		case code
		case bar(ActionControls.Place)
	}

	private static let summaryGap: CGFloat = 6
	private static let codeGap: CGFloat = 8
	/// Between the code box's edges and its text.
	private static let codeInset = NSSize(width: 9, height: 6)

	private let inbox: InboxModel
	private let summary = CrossfadingText { Label.wrapping(font: .systemFont(ofSize: 13), color: .secondaryLabelColor) }
	private let codeBox = FlippedView()
	private let code = CrossfadingText { Label.wrapping(font: .monospacedSystemFont(ofSize: 12, weight: .regular), maxLines: 4) }
	private let summarySlot = SlotView()
	private let codeSlot = SlotView()
	private let actionsSlot = SlotView()
	private let replyFieldSlot = SlotView()
	private let replyButtonsSlot = SlotView()
	private lazy var parts = SlotStack<Part>(in: self, width: 0)
	/// The parts the last layout listed.
	private var listedParts: Set<Part> = []
	private(set) var message: InboxMessage?
	/// The reply box is open on this card. While it is typed in, the field
	/// holds the text; the model keeps a copy, for a superseding message.
	private var isReplyOpen = false
	/// The lines the reply box took at the last layout.
	private var replyLines = 1
	/// The body changed by itself (a bar finished building or swapped in,
	/// the reply box took a line or gave one back): the row lays it out
	/// again as a change of the list.
	var onChange: (() -> Void)?
	/// The action bar's controls for the message's actions, shown.
	private var controls: ActionControls?
	/// Controls for actions that changed (a waiter arrived), built ahead and
	/// swapped in once complete.
	private var nextControls: ActionControls?
	/// Bars swapped out since the last layout, which fades them out.
	private var retiringBars: [ActionControls] = []
	/// Bars fading out, each taken out once its change has run.
	private var leavingBars: [Int: [ActionControls]] = [:]
	/// Each control's fade in or out as its bar comes or goes.
	private var controlFades: [ObjectIdentifier: ContentMotion] = [:]
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
			updateReach()
		}
	}

	private var keys = CardKeyGate()

	init(inbox: InboxModel) {
		self.inbox = inbox
		super.init(frame: .zero)
		codeBox.fill = Tokens.codeBox
		codeBox.cornerRadius = 6
		code.label.isSelectable = true
		summary.add(to: summarySlot.content)
		code.add(to: codeBox)
		codeSlot.content.addSubview(codeBox)
		setAccessibilityElement(false)
		setAccessibilityRole(.group)
	}

	/// Shows `message`; on a shown card, as change `motion`, a new message's
	/// text cross-fades from the one it replaces. The row lays the body out
	/// next.
	func show(message: InboxMessage, isReplying isReplyOpen: Bool, in motion: MotionChange?) {
		keys.show(messageId: message.id, isShown: isActive, at: CACurrentMediaTime())
		let isNewMessage = message != self.message
		if isNewMessage {
			self.message = message
			let time = isActive ? motion?.time : nil
			// A text that goes leaves with its part, as it showed.
			if let text = message.summary {
				summary.show(text, crossfadingAt: listedParts.contains(.summary) ? time : nil)
			}
			if let text = message.body {
				code.show(text, crossfadingAt: listedParts.contains(.code) ? time : nil)
			}
			prepareControls(for: ActionControls.Spec(message: message))
		}
		let wasReplying = isReplying
		self.isReplyOpen = isReplyOpen
		if isReplying != wasReplying {
			hasSent = false
			syncReplyField()
		} else if isNewMessage {
			// A superseding message carries the draft over.
			syncReplyField()
		}
	}

	/// Builds whatever controls are still missing now: the body is about to
	/// show. The row lays the body out next.
	func finishControls() {
		if let nextControls {
			buildAll(of: nextControls)
			swapIn(nextControls)
		}
		if let controls {
			buildAll(of: controls)
		}
	}

	/// Lays its parts out `width` wide and lists them, as change `motion` or
	/// at once; its height once the motion has run.
	@discardableResult
	func layOut(width: CGFloat, in motion: MotionChange?) -> CGFloat {
		parts.width = width
		let entries = layOutParts(width: width)
		let listed = Set(entries.map(\.id))
		guard let motion else {
			parts.settle(entries)
			endResizing()
			listedParts = listed
			syncControls(in: nil, steadyParts: listed)
			return parts.targetHeight
		}
		let applied = parts.apply(
			entries, change: motion.change, at: motion.time, duration: motion.duration, reducesMotion: motion.reducesMotion,
		)
		let comingOrGoing = applied.inserted.union(applied.removed).union(applied.returned)
		let steadyParts = listed.intersection(listedParts).subtracting(comingOrGoing)
		resizeInsides(by: applied, steadyParts: steadyParts)
		listedParts = listed
		syncControls(in: motion, steadyParts: steadyParts)
		return parts.targetHeight
	}

	func complete(_ change: Int) {
		parts.complete(change)
		for bar in leavingBars.removeValue(forKey: change) ?? [] {
			discard(bar)
		}
	}

	func settle() {
		parts.settle()
		for bar in retiringBars + leavingBars.values.flatMap({ $0 }) {
			discard(bar)
		}
		retiringBars = []
		leavingBars = [:]
		for view in controls?.allViews ?? [] {
			endFade(of: view, at: 1)
		}
		summary.end()
		code.end()
		endResizing()
		updateReach()
	}

	// MARK: Parts

	/// Each part laid out at its natural size from its top, as the stack
	/// lists it: the summary and the code box when the message has them,
	/// then the bar's row of buttons, or the reply field over its Send and
	/// Cancel.
	private func layOutParts(width: CGFloat) -> [SlotStack<Part>.Entry] {
		var entries: [SlotStack<Part>.Entry] = []
		if message?.summary != nil {
			entries.append(entry(.summary, height: layOutSummary(width: width), width: width))
		}
		if message?.body != nil {
			entries.append(entry(.code, height: layOutCode(width: width), width: width))
		}
		guard let controls else { return entries }
		for bar in [controls, nextControls].compactMap({ $0 }) {
			bar.layOut(width: width)
		}
		if isReplying, let field = controls.replyField {
			replyLines = field.lineCount(width: width)
		}
		for place in controls.places(isReplying: isReplying) {
			entries.append(entry(.bar(place), height: controls.height(of: place, width: width), width: width))
		}
		return entries
	}

	private func layOutSummary(width: CGFloat) -> CGFloat {
		let height = Label.height(of: summary.label, width: width)
		summary.label.frame = Label.frame(x: 0, y: Self.summaryGap, width: width, height: height)
		return Self.summaryGap + height
	}

	private func layOutCode(width: CGFloat) -> CGFloat {
		let inset = Self.codeInset
		let textHeight = Label.height(of: code.label, width: width - 2 * inset.width)
		codeBox.frame = NSRect(x: 0, y: Self.codeGap, width: width, height: textHeight + 2 * inset.height)
		code.label.frame = Label.frame(x: inset.width, y: inset.height, width: width - 2 * inset.width, height: textHeight)
		return Self.codeGap + codeBox.frame.height
	}

	private func entry(_ part: Part, height: CGFloat, width: CGFloat) -> SlotStack<Part>.Entry {
		let slot = slot(for: part)
		slot.content.frame = NSRect(x: 0, y: 0, width: width, height: height)
		return SlotStack.Entry(id: part, view: slot, height: height)
	}

	private func slot(for part: Part) -> SlotView {
		switch part {
		case .summary: summarySlot
		case .code: codeSlot
		case .bar(.actions): actionsSlot
		case .bar(.replyField): replyFieldSlot
		case .bar(.replyButtons): replyButtonsSlot
		}
	}

	/// The code box and the reply field fill their parts, which clip them:
	/// each changes height on its part's tracks, so its bottom edge moves
	/// with the part's.
	private func resizeInsides(by applied: MotionTimeline<Part>.Change, steadyParts: Set<Part>) {
		let insides: [(part: Part, layer: CALayer?)] = [(.code, codeBox.layer), (.bar(.replyField), controls?.replyField?.layer)]
		for inside in insides where steadyParts.contains(inside.part) {
			inside.layer?.addMotions(applied.heightTracks[inside.part] ?? [], keyPath: "bounds.size.height")
		}
	}

	private func endResizing() {
		for layer in [codeBox.layer, controls?.replyField?.layer] {
			layer?.removeMotions(keyedFrom: "motion.bounds.")
		}
	}

	// MARK: Controls

	private var isReplying: Bool {
		isReplyOpen && controls?.spec.reply != nil
	}

	private func prepareControls(for spec: ActionControls.Spec) {
		guard spec != controls?.spec else {
			if let nextControls {
				discard(nextControls)
			}
			nextControls = nil
			return
		}
		guard spec != nextControls?.spec else { return }
		if let nextControls {
			discard(nextControls)
		}
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
			guard let self, let bar, !bar.isComplete, bar === self.controls || bar === self.nextControls else { return }
			if self.buildNext(of: bar) {
				self.buildAhead(bar)
				return
			}
			if bar === self.nextControls {
				self.swapIn(bar)
			}
			self.changedByItself()
		}
	}

	/// Builds `bar`'s next control into its part, drawn but transparent and
	/// out of reach; whether more are left.
	@discardableResult
	private func buildNext(of bar: ActionControls) -> Bool {
		guard let view = bar.buildNext(for: self) else { return false }
		view.alphaValue = 0
		view.setInReach(false)
		slot(for: .bar(bar.place(of: view))).content.addSubview(view)
		return !bar.isComplete
	}

	private func buildAll(of bar: ActionControls) {
		while buildNext(of: bar) {}
	}

	/// The next layout fades the old bar out.
	private func swapIn(_ bar: ActionControls) {
		if let controls {
			retiringBars.append(controls)
		}
		controls = bar
		nextControls = nil
		syncReplyField()
	}

	/// A shown card lays out again as a change of the list; a hidden one at
	/// once.
	private func changedByItself() {
		if isActive {
			onChange?()
		} else if parts.width > 0 {
			layOut(width: parts.width, in: nil)
		}
	}

	/// The bar shows its views at full opacity. Where a bar replaces another
	/// in a part that stays, its views fade in growing slightly as the old
	/// bar's fade out shrinking, and the old bar goes once the change has
	/// run; in a part that comes or goes, the part's own fade shows them, so
	/// a control's fade on its way runs on.
	private func syncControls(in motion: MotionChange?, steadyParts: Set<Part>) {
		for view in controls?.allViews ?? [] {
			let isSteady = controls.map { steadyParts.contains(.bar($0.place(of: view))) } ?? false
			if let motion, isSteady {
				fade(view, to: 1, in: motion)
			} else if motion == nil || view.alphaValue != 1 {
				endFade(of: view, at: 1)
			}
		}
		let retiring = retiringBars
		retiringBars = []
		for bar in retiring {
			guard let motion else {
				discard(bar)
				continue
			}
			for view in bar.allViews where steadyParts.contains(.bar(bar.place(of: view))) {
				fade(view, to: 0, in: motion)
			}
			leavingBars[motion.change, default: []].append(bar)
		}
		updateReach()
	}

	/// A control fading in from slightly smaller, or out shrinking, from
	/// what it shows: turned back mid-fade, nothing jumps.
	private func fade(_ control: NSView, to opacity: CGFloat, in motion: MotionChange) {
		guard control.alphaValue != opacity else { return }
		let key = ObjectIdentifier(control)
		let target = MotionSpec.content(opacity == 1 ? .insert : .remove, reducesMotion: motion.reducesMotion)
		let running = controlFades[key]
		let spec = MotionSpec.Content(
			fromOpacity: running?.opacity(at: motion.time) ?? target.fromOpacity, toOpacity: target.toOpacity,
			fromScale: running?.scale(at: motion.time) ?? target.fromScale, toScale: target.toScale,
			duration: target.duration, delay: 0,
		)
		let fade = ContentMotion(spec: spec, start: motion.time)
		fade.commit(to: control)
		controlFades[key] = fade
	}

	private func endFade(of control: NSView, at opacity: CGFloat) {
		control.layer?.removeMotions(keyedFrom: "motion.content.")
		controlFades[ObjectIdentifier(control)] = nil
		control.alphaValue = opacity
	}

	private func discard(_ bar: ActionControls) {
		for view in bar.allViews {
			controlFades[ObjectIdentifier(view)] = nil
		}
		bar.remove()
	}

	private var allBars: [ActionControls] {
		[controls, nextControls].compactMap { $0 } + retiringBars + leavingBars.values.flatMap { $0 }
	}

	/// The views that take clicks, keys and VoiceOver: the shown bar's, on
	/// an active body.
	private var reachableViews: [NSView] {
		isActive ? controls?.views(isReplying: isReplying) ?? [] : []
	}

	/// Every other control stays out of reach of clicks, keys and VoiceOver;
	/// the reply field takes the keys once its part is listed.
	private func updateReach() {
		let reachable = reachableViews
		for view in allBars.flatMap(\.allViews) {
			view.setInReach(reachable.contains { $0 === view })
		}
		if isActive, isReplying, listedParts.contains(.bar(.replyField)), let field = controls?.replyField {
			if field.currentEditor() == nil {
				window?.makeFirstResponder(field)
			}
		} else if allBars.contains(where: { $0.replyField?.currentEditor() != nil }) {
			window?.makeFirstResponder(nil)
		}
	}

	/// The controls shown take clicks; the concealed ones never do, even
	/// where one lies over a shown one. `FlippedView` answers a concealed
	/// part with the body itself, so a hit on the body is looked at again
	/// too.
	override func hitTest(_ point: NSPoint) -> NSView? {
		guard isActive, let hit = super.hitTest(point) else { return nil }
		let reachable = reachableViews
		if reachable.contains(where: { hit === $0 || hit.isDescendant(of: $0) }) {
			return hit
		}
		for view in reachable.reversed() {
			guard let container = view.superview else { continue }
			if let reachableHit = view.hitTest(container.convert(point, from: superview)) {
				return reachableHit
			}
		}
		let barViews = allBars.flatMap(\.allViews)
		return barViews.contains { hit === $0 || hit.isDescendant(of: $0) } ? self : hit
	}

	override func performKeyEquivalent(with event: NSEvent) -> Bool {
		guard isActive, !event.isARepeat, keys.isOpen(at: CACurrentMediaTime()) else { return false }
		return reachableViews.contains { $0.performKeyEquivalent(with: event) }
	}

	override func viewDidMoveToWindow() {
		super.viewDidMoveToWindow()
		updateReach()
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
	/// The model's draft into the field: when the box opens or closes, a
	/// draft carries over or a new field takes the old one's place.
	private func syncReplyField() {
		guard let field = controls?.replyField else { return }
		let text = isReplyOpen ? inbox.replyDraft?.text ?? "" : ""
		if field.stringValue != text {
			field.stringValue = text
			// The caret after the text, as if typed.
			let end = NSRange(location: (text as NSString).length, length: 0)
			field.currentEditor()?.selectedRange = end
			field.currentEditor()?.scrollRangeToVisible(end)
		}
		controls?.send?.isEnabled = !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
	}

	/// A keystroke: the model keeps a copy of the text, and the card lays
	/// out again only when the box takes a line or gives one back.
	func controlTextDidChange(_: Notification) {
		guard let field = controls?.replyField else { return }
		inbox.updateReply(field.stringValue)
		controls?.send?.isEnabled = !field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
		let lines = field.lineCount(width: parts.width)
		if lines != replyLines {
			replyLines = lines
			changedByItself()
		}
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
/// and Cancel when the message takes a reply. Each shows in the body's part
/// for its place. Their actions go through the body, to the message it
/// shows now, so a superseding message with the same actions keeps them.
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

	/// The body's parts the bar shows in: the row of buttons, or the reply
	/// field over its Send and Cancel.
	enum Place: Hashable {
		case actions
		case replyField
		case replyButtons
	}

	/// Native large controls: the 28-point `.btn`.
	static let barHeight: CGFloat = 28
	/// Space above the bar.
	static let barGap: CGFloat = 12
	/// Space between the reply field and its buttons.
	static let replyGap: CGFloat = 8

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

	private var hasReplyViews: Bool {
		replyField != nil && send != nil && cancel != nil
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

	/// The places the bar shows in, top to bottom.
	func places(isReplying: Bool) -> [Place] {
		isReplying && hasReplyViews ? [.replyField, .replyButtons] : [.actions]
	}

	func place(of view: NSView) -> Place {
		if view === replyField {
			return .replyField
		}
		if view === send || view === cancel {
			return .replyButtons
		}
		return .actions
	}

	/// Builds the next control for `body`, for the body to place; none once
	/// the bar is complete.
	func buildNext(for body: CardBodyView) -> NSView? {
		guard !factories.isEmpty else { return nil }
		return factories.removeFirst()(self, body)
	}

	func remove() {
		allViews.forEach { $0.removeFromSuperview() }
	}

	/// The height of `place`'s part, the space above it included.
	func height(of place: Place, width: CGFloat) -> CGFloat {
		switch place {
		case .actions: Self.barGap + Self.barHeight
		case .replyField: Self.barGap + (replyField?.height(width: width) ?? 0)
		case .replyButtons: Self.replyGap + Self.barHeight
		}
	}

	/// Each control in its part, from the part's top: the buttons left to
	/// right, the reply field across the width.
	func layOut(width: CGFloat) {
		if let replyField {
			let frame = NSRect(x: 0, y: Self.barGap, width: width, height: replyField.height(width: width))
			if replyField.frame != frame {
				replyField.frame = frame
			}
		}
		let rows: [(views: [NSView], top: CGFloat)] = [
			(answerButtons, Self.barGap), ([fallbackButton].compactMap { $0 }, Self.barGap),
			([send, cancel].compactMap { $0 }, Self.replyGap),
		]
		for row in rows {
			var x: CGFloat = 0
			for view in row.views {
				let top = row.top + ((Self.barHeight - view.frame.height) / 2).rounded()
				view.move(to: NSPoint(x: x, y: top))
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
