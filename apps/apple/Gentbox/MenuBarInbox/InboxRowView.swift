import AppKit
import GentboxCore

/// One item of the accordion (design: `mac-popover`: "the active card keeps
/// a row's order and adds its body below, so a row can grow into it").
/// Collapsed, it is a two-line row: type icon, title, breadcrumb, and the
/// waiting mark, which the ellipsis menu replaces on hover. Expanded, the
/// same header, with the same padding, has the body and buttons revealed
/// below it. Its text keeps the row's sizes (the user's decision):
/// expanding adds content and actions and enlarges nothing. Only the
/// expanded item has buttons, so ↩, ⌘⌫ and R act on it alone. A message
/// that replaces the one shown cross-fades into its place.
///
/// A slot of the list (ADR 0013) and a stack of its own: the rule above it,
/// its header and its body are its parts, so a row that grows into a card
/// opens its body's place on the list's clock, and the body fades in once
/// the place is made.
///
/// For VoiceOver a collapsed item is one element: its title, breadcrumb,
/// agent and waiting mark in one label, "Muted" as its value, and a press
/// that expands it. The card contains its buttons and field.
///
/// A click on a collapsed item's header expands it (and collapses the open
/// one); a click on the expanded item's header leaves it expanded, as one
/// card is always open, and sends nothing yet (Phase 4 makes it open the
/// agent's session). ⌘-click picks a collapsed
/// item for a bulk Dismiss (never an `action_required` one); while any is
/// picked, every row that can be picked shows a checkbox in place of its
/// icon, and a click on it picks or unpicks the row.
@MainActor
final class InboxRowView: SlotView, MotionStack {
	enum Part: Hashable {
		case rule
		case header
		case body
	}

	/// The mark for a message an agent waits on, and what VoiceOver reads.
	private struct WaitingMark: Equatable {
		let symbol: Symbol.Shown
		let label: String
	}

	/// The row itself, for the list's stack: unlike the message it shows,
	/// it never changes, and unlike an object identifier, never comes back.
	let rowId: Int
	private static var rowsMade = 0
	/// The message it shows; a message that supersedes it takes the row over.
	private(set) var messageId: String
	private(set) var rowContent: RowContent
	private let inbox: InboxModel
	/// The rule above every row but the first, the header and the card's
	/// body, moved on the list's clock.
	private lazy var parts = SlotStack<Part>(in: content, width: Tokens.popoverWidth)
	private let ruleSlot = SlotView()
	private let headerSlot = SlotView()
	private let bodySlot = SlotView()
	/// The header's leaves cross-fade to a new message's on the list's clock,
	/// each alone, so nothing behind them flickers.
	private let icon = Crossfading<NSImageView, Symbol.Shown>(InboxRowView.symbolView, write: Symbol.set)
	/// In place of the icon while rows are being picked; it shows whether
	/// the row is picked.
	private let checkbox = Crossfading<NSImageView, Bool>(InboxRowView.symbolView) { view, isPicked in
		Symbol.set(view, isPicked ? InboxRowView.checkboxSymbols[1] : InboxRowView.checkboxSymbols[0], size: InboxRowView.checkboxSize, color: isPicked ? .controlAccentColor : .tertiaryLabelColor)
	}
	/// Whether the message is urgent.
	private let urgentBadge = Crossfading<NSImageView, Bool>({ Symbol.view("figure.walk.motion", size: 12, color: Tokens.waiting) }) { view, isUrgent in
		view.isHidden = !isUrgent
	}
	private let title = CrossfadingText { Label.line(size: 13, weight: .semibold) }
	private let context = Crossfading<ContextLine, ContextLine.Shown>({ ContextLine() }) { line, shown in line.show(shown) }
	private let waitingMark = Crossfading<NSImageView, WaitingMark?>(InboxRowView.symbolView) { view, mark in
		if let mark {
			Symbol.set(view, mark.symbol)
			view.setAccessibilityLabel(mark.label)
		}
		view.isHidden = mark == nil
	}
	/// Built with the row and drawn at zero opacity until a hover: a hover
	/// builds nothing in its frame. It pops up the item menu, built only
	/// then.
	private let moreButton = MoreButton()
	/// Built ahead (`prepareBody`) and kept while collapsed, its place
	/// closed and transparent, so expanding only reveals it.
	private var body: CardBodyView?
	private var isHovered = false {
		didSet { if isHovered != oldValue { showHover() } }
	}

	private var isPressInHeader = false
	private var hasShownMessage = false
	/// The row's height once its parts' motion has run.
	private(set) var height: CGFloat = 0
	/// From the row's top to the header's bottom, the rule included.
	private(set) var headerHeight: CGFloat = 0

	/// Leaving, or parked while the hub confirms an answer: its card takes
	/// no clicks, keys or VoiceOver.
	override var isLeaving: Bool {
		didSet {
			body?.isActive = !isLeaving && rowContent.isExpanded
			setAccessibilityElement(!isLeaving)
		}
	}

	init(content: RowContent, inbox: InboxModel) {
		Self.rowsMade += 1
		rowId = Self.rowsMade
		messageId = content.message.id
		rowContent = content
		self.inbox = inbox
		super.init(frame: NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: 0))
		fill = Tokens.row
		ruleSlot.content.fill = Tokens.line
		ruleSlot.content.frame = NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: 1)
		// Decorative: `checkmark.circle` would read "Selected" and carry the
		// Selected trait, which bulk Dismiss's picks use.
		icon.leaf.setAccessibilityElement(false)
		// The row's "Pick for Dismiss" action and Selected trait speak for it.
		checkbox.leaf.setAccessibilityElement(false)
		checkbox.leaf.alphaValue = 0
		urgentBadge.leaf.setAccessibilityLabel("Urgent")
		// Their ghosts are made ahead with the card's body (`prepareBody`):
		// a row made in a change's frame makes no more than it shows.
		icon.add(to: headerSlot.content, ghostsAhead: 0)
		checkbox.add(to: headerSlot.content, ghostsAhead: 0)
		urgentBadge.add(to: headerSlot.content, ghostsAhead: 0)
		title.add(to: headerSlot.content, ghostsAhead: 0)
		context.add(to: headerSlot.content, ghostsAhead: 0)
		waitingMark.add(to: headerSlot.content, ghostsAhead: 0)
		setUpMoreButton()
		addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
		apply(content, in: nil)
	}

	/// The header's leaves that cross-fade to a new message's; the checkbox
	/// only fades.
	private var crossfadingLeaves: [any CrossfadingPart] {
		[icon, urgentBadge, title, context, waitingMark]
	}

	/// Shows `newContent`, moving its parts as change `motion`, or at once
	/// without one; the caller places the row.
	func apply(_ newContent: RowContent, in motion: MotionChange?) {
		if hasShownMessage, newContent.differsOnlyInPicking(from: rowContent) {
			// A pick changes the tint and the checkbox, nothing laid out: the
			// first pick touches every row in its click's frame.
			rowContent = newContent
			showTint()
			showCheckbox(in: motion)
			describeForAccessibility()
			return
		}
		let wasExpanded = rowContent.isExpanded
		let isNewMessage = !hasShownMessage || newContent.message != rowContent.message
		rowContent = newContent
		messageId = newContent.message.id
		let message = newContent.message
		// Before the header cross-fades: the mark fades in to it.
		waitingMark.fade(to: isHovered && !newContent.isExpanded ? 0 : 1, at: nil)
		if isNewMessage {
			show(message, crossfadingAt: hasShownMessage ? motion?.time : nil)
		}
		showTint()
		showCheckbox(in: motion)
		if newContent.isExpanded, body == nil {
			makeBody()
		}
		body?.show(message: message, isReplying: newContent.isReplying, in: motion)
		if newContent.isExpanded {
			body?.finishControls()
		}
		body?.isActive = newContent.isExpanded && !isLeaving
		if !newContent.isExpanded, isHovered {
			showHover()
		}
		moreButton.alphaValue = newContent.isExpanded || !isHovered ? 0 : 1
		listParts(in: motion, wasExpanded: wasExpanded)
		describeForAccessibility()
	}

	/// Shows `message` in the header: cross-fading from the message shown
	/// at `time`, at once without a time.
	private func show(_ message: InboxMessage, crossfadingAt time: TimeInterval?) {
		hasShownMessage = true
		let typeSymbol = Symbol.Shown(name: InboxWords.typeSymbol(for: message), size: 14, color: Self.color(for: message.messageType))
		icon.show(typeSymbol, crossfadingAt: time)
		urgentBadge.show(message.isUrgent, crossfadingAt: time)
		title.show(message.title, crossfadingAt: time)
		context.show(ContextLine.Shown(message), crossfadingAt: time)
		// On the line itself: a ghost fading out is no element.
		context.leaf.setAccessibilityIdentifier("context:\(message.title)")
		waitingMark.show(Self.waitingMark(for: message), crossfadingAt: time)
		moreButton.setAccessibilityIdentifier("message-menu:\(message.title)")
	}

	private static func waitingMark(for message: InboxMessage) -> WaitingMark? {
		guard let label = InboxWords.waitingLabel(for: message.blocking) else { return nil }
		let color = message.blocking == .blocking ? Tokens.waiting : Tokens.paused
		return WaitingMark(symbol: Symbol.Shown(name: "pause.circle", size: 14, color: color), label: label)
	}

	/// An image view that draws its symbol at the symbol's size.
	private static func symbolView() -> NSImageView {
		let view = NSImageView()
		view.imageScaling = .scaleNone
		return view
	}

	/// The checkbox empty and checked, made ahead by the list.
	static let checkboxSymbols = ["circle", "checkmark.circle.fill"]
	static let checkboxSize: CGFloat = 15

	/// Whether this row can be picked for a bulk Dismiss now.
	private var canPick: Bool {
		!rowContent.isExpanded && inbox.canSelect(rowContent.message)
	}

	/// While rows are being picked: an empty circle in place of the type
	/// icon, filled with a check once picked. The icon and the checkbox
	/// cross-fade as picking starts or ends; a pick shows at once.
	private func showCheckbox(in motion: MotionChange?) {
		let showsCheckbox = rowContent.isSelecting && canPick
		if showsCheckbox {
			checkbox.update(rowContent.isPicked)
		}
		checkbox.fade(to: showsCheckbox ? 1 : 0, at: motion?.time)
		icon.fade(to: showsCheckbox ? 0 : 1, at: motion?.time)
		urgentBadge.fade(to: showsCheckbox ? 0 : 1, at: motion?.time)
	}

	/// Where a click picks the row while rows are being picked: the icon's
	/// column, the header's height.
	private var checkboxTarget: NSRect {
		NSRect(x: 0, y: 0, width: RowMetrics.textX - Tokens.columnGap / 2, height: headerHeight)
	}

	/// The minute changed: "4 min ago".
	func refreshTime() {
		context.update(ContextLine.Shown(rowContent.message))
	}

	/// Drops the row's motion and its leaves' fades: they show their model.
	override func endMotion() {
		super.endMotion()
		for leaf in crossfadingLeaves {
			leaf.end()
		}
		checkbox.end()
		moreButton.layer?.removeMotions(keyedFrom: "motion.")
	}

	// MARK: Layout

	/// Lays the parts out and lists them: the rule, the header and, once
	/// built, the body, whose place is closed while the row is collapsed.
	private func listParts(in motion: MotionChange?, wasExpanded: Bool) {
		let headerSlotHeight = layOutHeader()
		let bodyHeight = layOutBody(in: motion)
		let ruleHeight: CGFloat = rowContent.isFirst ? 0 : 1
		headerHeight = ruleHeight + headerSlotHeight
		var entries: [SlotStack<Part>.Entry] = []
		if !rowContent.isFirst {
			entries.append(SlotStack.Entry(id: .rule, view: ruleSlot, height: ruleHeight))
		}
		entries.append(SlotStack.Entry(id: .header, view: headerSlot, height: headerSlotHeight))
		if body != nil {
			entries.append(SlotStack.Entry(id: .body, view: bodySlot, height: rowContent.isExpanded ? bodyHeight : 0))
		}
		if let motion {
			let applied = parts.apply(
				entries, change: motion.change, at: motion.time, duration: motion.duration, reducesMotion: motion.reducesMotion,
			)
			fadeBody(wasExpanded: wasExpanded, isInserted: applied.inserted.contains(.body), in: motion)
		} else {
			parts.settle(entries)
			bodySlot.content.alphaValue = rowContent.isExpanded ? 1 : 0
		}
		// Up to the device pixel, as SwiftUI laid rows out.
		let pixels = window?.backingScaleFactor ?? 2
		height = ((parts.targetHeight + RowMetrics.bottomPadding) * pixels).rounded(.up) / pixels
		content.frame = NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: height)
	}

	/// The header's leaves from the header's top; its height.
	private func layOutHeader() -> CGFloat {
		let isExpanded = rowContent.isExpanded
		let top = RowMetrics.topPadding
		let titleHeight = RowMetrics.titleLineHeight
		title.leaf.frame = Label.frame(x: RowMetrics.textX, y: top, width: RowMetrics.textWidth, height: titleHeight)
		let contextTop = top + titleHeight + RowMetrics.titleGap
		context.leaf.frame = NSRect(x: RowMetrics.textX, y: contextTop, width: RowMetrics.textWidth, height: RowMetrics.contextHeight)
		context.leaf.layOut()

		let iconSize = icon.leaf.image?.size ?? NSSize(width: 16, height: 16)
		icon.leaf.frame = NSRect(
			x: Tokens.rowHorizontalPadding + (Tokens.iconColumn - iconSize.width) / 2, y: top + 1,
			width: iconSize.width, height: iconSize.height,
		)
		// A fixed box, whichever symbol it shows: a pick changes the symbol
		// without laying the row out.
		let box: CGFloat = 20
		checkbox.leaf.frame = NSRect(
			x: Tokens.rowHorizontalPadding + (Tokens.iconColumn - box) / 2, y: (icon.leaf.frame.midY - box / 2).rounded(),
			width: box, height: box,
		)
		let badgeSize = urgentBadge.leaf.image?.size ?? .zero
		urgentBadge.leaf.frame = NSRect(
			x: Tokens.rowHorizontalPadding + (Tokens.iconColumn - badgeSize.width) / 2, y: icon.leaf.frame.maxY + 3,
			width: badgeSize.width, height: badgeSize.height,
		)
		let iconBottom = urgentBadge.leaf.isHidden ? icon.leaf.frame.maxY : urgentBadge.leaf.frame.maxY

		let markX = Tokens.popoverWidth - Tokens.rowHorizontalPadding - RowMetrics.markWidth
		let markSize = waitingMark.leaf.image?.size ?? .zero
		waitingMark.leaf.frame = NSRect(
			x: markX + (RowMetrics.markWidth - markSize.width) / 2, y: top + (titleHeight - markSize.height) / 2,
			width: markSize.width, height: markSize.height,
		)
		moreButton.frame = NSRect(
			x: markX, y: top + (titleHeight - RowMetrics.markWidth) / 2, width: RowMetrics.markWidth, height: RowMetrics.markWidth,
		)
		let markBottom = isExpanded ? waitingMark.leaf.frame.maxY : top + max(titleHeight, RowMetrics.markWidth)
		let headerBottom = max(context.leaf.frame.maxY, iconBottom, markBottom)
		headerSlot.content.frame = NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: headerBottom)
		return headerBottom
	}

	/// The body at its natural height, from its slot's top, its own parts
	/// moved as change `motion`; that height.
	private func layOutBody(in motion: MotionChange?) -> CGFloat {
		guard let body else { return 0 }
		let bodyHeight = body.layOut(width: RowMetrics.bodyWidth, in: motion)
		body.frame = NSRect(x: RowMetrics.textX, y: 0, width: RowMetrics.bodyWidth, height: bodyHeight)
		bodySlot.content.frame = NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: bodyHeight)
		return bodyHeight
	}

	/// Expanded, the body fades in over the second half of the motion, once
	/// its place is made, so it never draws over a moving header; collapsed,
	/// it fades out at once, before the rows below slide up, and waits
	/// transparent for the next expand. Turned back mid-fade, it starts
	/// from what it shows.
	private func fadeBody(wasExpanded: Bool, isInserted: Bool, in motion: MotionChange) {
		let isExpanded = rowContent.isExpanded
		guard isExpanded != wasExpanded || isInserted else { return }
		let shown = bodySlot.shownContent(at: motion.time).opacity
		let isFading = bodySlot.contentMotion.map { fade in
			motion.time < fade.start + fade.spec.delay + fade.spec.duration
		} ?? false
		let spec: MotionSpec.Content
		switch (isExpanded, isFading) {
		case (true, false):
			spec = MotionSpec.content(.expand, reducesMotion: motion.reducesMotion)
		case (true, true):
			spec = MotionSpec.Content(
				fromOpacity: shown, toOpacity: 1, fromScale: 1, toScale: 1, duration: Tokens.motionDuration / 2, delay: 0,
			)
		case (false, _):
			let collapse = MotionSpec.content(.collapse, reducesMotion: motion.reducesMotion)
			// A body built collapsed waits transparent from the start.
			spec = MotionSpec.Content(
				fromOpacity: isInserted && !wasExpanded ? 0 : shown, toOpacity: 0, fromScale: 1, toScale: 1,
				duration: collapse.duration, delay: 0,
			)
		}
		bodySlot.moveContent(spec, at: motion.time)
	}

	private func showTint() {
		if rowContent.isPicked {
			content.fill = RowMetrics.pickedFill
		} else {
			content.fill = isHovered && !rowContent.isExpanded ? RowMetrics.hoverFill : nil
		}
	}

	// MARK: The card's body

	/// Builds the card's body before anyone expands the row, its controls a
	/// piece at a time (`IdleWork`, which runs no piece while anything
	/// moves): its place, closed, is listed at once, so it draws ahead. The
	/// header's ghosts come first, for a message that supersedes this one.
	func prepareBody() {
		for leaf in crossfadingLeaves {
			leaf.makeGhostAhead()
		}
		guard body == nil else { return }
		makeBody()
		body?.show(message: rowContent.message, isReplying: rowContent.isReplying, in: nil)
		listParts(in: nil, wasExpanded: rowContent.isExpanded)
	}

	var hasBody: Bool {
		body != nil
	}

	/// The card's body changed by itself (its bar swapped in, the reply box
	/// took a line): the caller lays the row out again as a change of the
	/// list.
	var onBodyChange: ((InboxRowView) -> Void)?

	/// Lists the parts again for a body that changed by itself.
	func refreshLayout(in motion: MotionChange?) {
		listParts(in: motion, wasExpanded: rowContent.isExpanded)
	}

	/// The row's parts and its body's, one stack to the coordinator.
	func complete(_ change: Int) {
		parts.complete(change)
		body?.complete(change)
	}

	func settle() {
		parts.settle()
		body?.settle()
	}

	private func makeBody() {
		let body = CardBodyView(inbox: inbox)
		body.onChange = { [weak self] in
			guard let self else { return }
			self.onBodyChange?(self)
		}
		bodySlot.content.addSubview(body)
		self.body = body
	}

	// MARK: Hover, clicks and the menu

	override func mouseEntered(with _: NSEvent) {
		isHovered = true
	}

	override func mouseExited(with _: NSEvent) {
		isHovered = false
	}

	private func showHover() {
		showTint()
		let showsMore = isHovered && !rowContent.isExpanded
		moreButton.alphaValue = showsMore ? 1 : 0
		waitingMark.fade(to: showsMore ? 0 : 1, at: nil)
	}

	private func setUpMoreButton() {
		moreButton.onPress = { [weak self] button in
			guard let self else { return }
			let menu = ItemMenu.make(for: self.rowContent.message, moveTargets: self.rowContent.moveTargets, inbox: self.inbox)
			menu.popUp(positioning: nil, at: NSPoint(x: 0, y: button.bounds.height + 2), in: button)
		}
		// The row's context menu is the same menu for VoiceOver.
		moreButton.setAccessibilityElement(false)
		moreButton.alphaValue = 0
		headerSlot.content.addSubview(moreButton)
	}

	override func menu(for _: NSEvent) -> NSMenu? {
		ItemMenu.make(for: rowContent.message, moveTargets: rowContent.moveTargets, inbox: inbox)
	}

	override func acceptsFirstMouse(for _: NSEvent?) -> Bool {
		true
	}

	/// A row on its way out, or parked while the hub confirms an answer,
	/// keeps its card's controls but answers no keys: ↩ belongs to the card
	/// that shows.
	override func performKeyEquivalent(with event: NSEvent) -> Bool {
		guard !isLeaving, rowContent.isExpanded else { return false }
		return super.performKeyEquivalent(with: event)
	}

	/// The header is one target: its labels and icons take no clicks.
	override func hitTest(_ point: NSPoint) -> NSView? {
		guard let superview, !isHidden, !isLeaving else { return nil }
		let local = convert(point, from: superview)
		guard bounds.contains(local) else { return nil }
		if local.y < headerHeight {
			if moreButton.alphaValue > 0, convert(moreButton.frame, from: moreButton.superview).contains(local) {
				return moreButton
			}
			return self
		}
		return super.hitTest(point)
	}

	override func mouseDown(with event: NSEvent) {
		isPressInHeader = convert(event.locationInWindow, from: nil).y < headerHeight
	}

	override func mouseUp(with event: NSEvent) {
		let location = convert(event.locationInWindow, from: nil)
		guard isPressInHeader, bounds.contains(location), location.y < headerHeight else { return }
		isPressInHeader = false
		let isOnCheckbox = rowContent.isSelecting && canPick && checkboxTarget.contains(location)
		if isOnCheckbox || (canPick && event.modifierFlags.contains(.command)) {
			Interactions.untilNextFrame("Select")
			inbox.toggleSelection(of: rowContent.message)
		} else {
			activate()
		}
	}

	/// A collapsed row expands; the expanded card stays (`InboxModel.
	/// activateExpandedCard` says why it sends nothing yet).
	private func activate() {
		if rowContent.isExpanded {
			inbox.activateExpandedCard(rowContent.message)
		} else {
			Interactions.untilNextFrame("Expand")
			inbox.expand(messageId)
		}
	}

	// MARK: Accessibility

	private func describeForAccessibility() {
		let message = rowContent.message
		setAccessibilityElement(!isLeaving)
		setAccessibilityRole(rowContent.isExpanded ? .group : .button)
		setAccessibilityIdentifier(rowContent.isExpanded ? "card:\(message.title)" : "message:\(message.title)")
		// An expanded card's title is read by its own label inside it.
		setAccessibilityLabel(rowContent.isExpanded ? nil : collapsedLabel)
		setAccessibilityValue(InboxWords.isMuted(message) ? "Muted" : "")
		setAccessibilitySelected(rowContent.isPicked)
		// ⌘-click picks a row for bulk Dismiss; VoiceOver picks it here.
		setAccessibilityCustomActions(canPick ? [
			NSAccessibilityCustomAction(name: rowContent.isPicked ? "Unpick" : "Pick for Dismiss") { [weak self] in
				MainActor.assumeIsolated {
					guard let self else { return false }
					self.inbox.toggleSelection(of: self.rowContent.message)
					return true
				}
			},
		] : [])
	}

	/// VoiceOver's "show menu": the item menu the ellipsis and a right-click
	/// open.
	override func accessibilityPerformShowMenu() -> Bool {
		let menu = ItemMenu.make(for: rowContent.message, moveTargets: rowContent.moveTargets, inbox: inbox)
		menu.popUp(positioning: nil, at: NSPoint(x: RowMetrics.textX, y: headerHeight), in: self)
		return true
	}

	/// Title, breadcrumb, agent, time and waiting mark, as VoiceOver reads a
	/// combined row.
	private var collapsedLabel: String {
		let message = rowContent.message
		return [
			message.isUrgent ? "Urgent" : nil,
			message.title,
			context.leaf.spokenText,
			InboxWords.waitingLabel(for: message.blocking),
		].compactMap { $0 }.joined(separator: ", ")
	}

	override func accessibilityChildren() -> [Any]? {
		rowContent.isExpanded && !isLeaving ? super.accessibilityChildren() : []
	}

	override func accessibilityPerformPress() -> Bool {
		activate()
		return true
	}

	static func color(for type: MessageType) -> NSColor {
		switch type {
		case .decisionRequired: Tokens.decision
		case .actionRequired: Tokens.action
		case .taskCompleted: Tokens.done
		case .generic: Tokens.info
		}
	}
}


/// A row's measures, from the design's `.pp-line` and `.pp-hero`.
@MainActor
enum RowMetrics {
	/// The title's line box as a label reports it, as SwiftUI's `Text` did:
	/// 13-point semibold, on a row and on the card alike.
	static let titleLineHeight: CGFloat = Label.size(of: Label.line("Hg", size: 13, weight: .semibold)).height
	static let contextFont = NSFont.systemFont(ofSize: 11)
	static let dotFont = NSFont.systemFont(ofSize: 14, weight: .bold)
	static let contextHeight = max(lineHeight(of: contextFont), lineHeight(of: dotFont))
	static let textX = Tokens.rowHorizontalPadding + Tokens.iconColumn + Tokens.columnGap
	/// A row's padding above its title and below its last line, collapsed or
	/// expanded alike; and the space between the title and the breadcrumb.
	static let topPadding: CGFloat = 14
	static let bottomPadding: CGFloat = 16
	static let titleGap: CGFloat = 2
	/// The waiting mark's column, where the hover menu shows.
	static let markWidth: CGFloat = 22
	static let textWidth = Tokens.popoverWidth - textX - Tokens.columnGap - markWidth - Tokens.rowHorizontalPadding
	static let bodyWidth = Tokens.popoverWidth - textX - Tokens.rowHorizontalPadding

	/// The card colour under the hover tint and the pick tint.
	static let hoverFill = NSColor(name: "GentboxRowHover") { appearance in blend(Tokens.hover, over: Tokens.row, in: appearance) }
	static let pickedFill = NSColor(name: "GentboxRowPicked") { appearance in blend(Tokens.picked, over: Tokens.row, in: appearance) }

	static func lineHeight(of font: NSFont) -> CGFloat {
		ceil(font.ascender - font.descender + font.leading)
	}

	private static func blend(_ tint: NSColor, over base: NSColor, in appearance: NSAppearance) -> NSColor {
		var result = base
		appearance.performAsCurrentDrawingAppearance {
			guard let tint = tint.usingColorSpace(.sRGB), let base = base.usingColorSpace(.sRGB) else { return }
			result = base.blended(withFraction: tint.alphaComponent, of: tint.withAlphaComponent(1)) ?? base
		}
		return result
	}
}

/// A row's second line: "Topic › Thread · [agent] 4 min ago", with a faded
/// dot. One element for VoiceOver.
@MainActor
final class ContextLine: NSView {
	private let breadcrumb = Label.line(size: 11, color: .secondaryLabelColor)
	private let dot = Label.line("·", size: 14, weight: .bold, color: Tokens.dot)
	private let agent = NSImageView()
	private let time = Label.line(size: 11, color: .secondaryLabelColor)
	private var agentName = ""

	/// What the line shows of a message.
	struct Shown: Equatable {
		let breadcrumb: String?
		let agentMark: InboxWords.AgentMark
		let agentName: String
		let timeAgo: String

		init(_ message: InboxMessage) {
			breadcrumb = InboxWords.breadcrumb(for: message)
			agentMark = InboxWords.agentMark(for: message.agent)
			agentName = message.agentName
			timeAgo = InboxWords.timeAgo(message.receivedAt)
		}
	}

	override init(frame: NSRect) {
		super.init(frame: frame)
		agent.imageScaling = .scaleProportionallyUpOrDown
		for view in [breadcrumb, dot, agent, time] as [NSView] {
			view.setAccessibilityElement(false)
			addSubview(view)
		}
		setAccessibilityElement(true)
		setAccessibilityRole(.staticText)
	}

	@available(*, unavailable)
	required init?(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	override var isFlipped: Bool { true }

	/// What VoiceOver reads: breadcrumb, agent, time.
	var spokenText: String {
		[breadcrumb.isHidden ? nil : breadcrumb.stringValue, agentName, time.stringValue]
			.compactMap { $0 }.joined(separator: ", ")
	}

	func show(_ shown: Shown) {
		breadcrumb.stringValue = shown.breadcrumb ?? ""
		breadcrumb.isHidden = shown.breadcrumb == nil
		dot.isHidden = shown.breadcrumb == nil
		agentName = shown.agentName
		switch shown.agentMark {
		case .claude:
			agent.image = NSImage(named: "AgentClaude")
			agent.contentTintColor = Tokens.claude
		case .codex:
			agent.image = NSImage(named: "AgentCodex")
			agent.contentTintColor = .secondaryLabelColor
		case .other:
			agent.image = NSImage(systemSymbolName: "terminal", accessibilityDescription: nil)
			agent.contentTintColor = .secondaryLabelColor
		}
		time.stringValue = shown.timeAgo
		setAccessibilityValue(spokenText)
		layOut()
	}

	/// Left to right, centred on the line: the breadcrumb gives way first.
	func layOut() {
		let height = bounds.height
		let spacing: CGFloat = 4
		let timeWidth = Label.textWidth(of: time)
		let dotWidth = Label.textWidth(of: dot)
		let agentSize: CGFloat = 11
		var x: CGFloat = 0
		if !breadcrumb.isHidden {
			let fixed = spacing + dotWidth + spacing + agentSize + spacing + timeWidth
			let width = min(Label.textWidth(of: breadcrumb), max(bounds.width - fixed, 0))
			breadcrumb.frame = centred(x: x, width: width, height: Label.size(of: breadcrumb).height, in: height)
			x += width + spacing
			dot.frame = centred(x: x, width: dotWidth, height: Label.size(of: dot).height, in: height)
			x += dotWidth + spacing
		}
		agent.frame = NSRect(x: x, y: ((height - agentSize) / 2).rounded(), width: agentSize, height: agentSize)
		x += agentSize + spacing
		time.frame = centred(x: x, width: timeWidth, height: Label.size(of: time).height, in: height)
	}

	/// A label's frame for text at `x`, `width` wide, centred on the line.
	private func centred(x: CGFloat, width: CGFloat, height: CGFloat, in lineHeight: CGFloat) -> NSRect {
		Label.frame(x: x, y: ((lineHeight - height) / 2).rounded(), width: width, height: height)
	}
}

/// The ellipsis on a hovered row, which opens the item menu.
@MainActor
private final class MoreButton: NSButton {
	var onPress: ((NSButton) -> Void)?

	init() {
		super.init(frame: .zero)
		bezelStyle = .accessoryBarAction
		image = NSImage(systemSymbolName: "ellipsis", accessibilityDescription: nil)
		imagePosition = .imageOnly
		target = self
		action = #selector(press)
	}

	@available(*, unavailable)
	required init?(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	override func acceptsFirstMouse(for _: NSEvent?) -> Bool {
		true
	}

	@objc private func press() {
		onPress?(self)
	}
}
