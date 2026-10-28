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
	private let icon = NSImageView()
	/// In place of the icon while rows are being picked.
	private let checkbox = NSImageView()
	private let urgentBadge = Symbol.view("figure.walk.motion", size: 12, color: Tokens.waiting)
	private let title = Label.line(size: 13, weight: .semibold)
	private let context = ContextLine()
	private let waitingMark = NSImageView()
	/// Built ahead with the body and kept drawn at zero opacity until a
	/// hover; it pops up the item menu, built only then.
	private var moreButton: NSButton?
	/// Built ahead (`prepareBody`) and kept while collapsed, its place
	/// closed and transparent, so expanding only reveals it.
	private var body: CardBodyView?
	/// The icon's and the checkbox's fades as picking starts or ends.
	private var leafFades: [ObjectIdentifier: ContentMotion] = [:]
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
		icon.imageScaling = .scaleNone
		// Decorative: `checkmark.circle` would read "Selected" and carry the
		// Selected trait, which bulk Dismiss's picks use.
		icon.setAccessibilityElement(false)
		checkbox.imageScaling = .scaleNone
		// The row's "Pick for Dismiss" action and Selected trait speak for it.
		checkbox.setAccessibilityElement(false)
		checkbox.alphaValue = 0
		urgentBadge.setAccessibilityLabel("Urgent")
		waitingMark.imageScaling = .scaleNone
		for view in headerLeaves {
			view.wantsLayer = true
			headerSlot.content.addSubview(view)
		}
		addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
		apply(content, in: nil)
	}

	private var headerLeaves: [NSView] {
		[icon, checkbox, urgentBadge, title, context, waitingMark]
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
		if isNewMessage, hasShownMessage, let motion {
			crossfadeHeader(at: motion.time)
		}
		rowContent = newContent
		messageId = newContent.message.id
		let message = newContent.message
		if isNewMessage {
			show(message)
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
		moreButton?.alphaValue = newContent.isExpanded || !isHovered ? 0 : 1
		waitingMark.alphaValue = isHovered && !newContent.isExpanded ? 0 : 1
		listParts(in: motion, wasExpanded: wasExpanded)
		describeForAccessibility()
	}

	private func show(_ message: InboxMessage) {
		hasShownMessage = true
		Symbol.set(icon, InboxWords.typeSymbol(for: message), size: 14, color: Self.color(for: message.messageType))
		urgentBadge.isHidden = !message.isUrgent
		title.stringValue = message.title
		context.show(message)
		if let label = InboxWords.waitingLabel(for: message.blocking) {
			Symbol.set(waitingMark, "pause.circle", size: 14, color: message.blocking == .blocking ? Tokens.waiting : Tokens.paused)
			waitingMark.setAccessibilityLabel(label)
			waitingMark.isHidden = false
		} else {
			waitingMark.isHidden = true
		}
	}

	/// A new message's header cross-fades from the old one's: its leaves
	/// alone, so nothing behind them flickers.
	private func crossfadeHeader(at time: TimeInterval) {
		for layer in [icon, urgentBadge, title, context, waitingMark].compactMap(\.layer) {
			let transition = CATransition()
			transition.type = .fade
			transition.beginTime = layer.convertTime(time, from: nil)
			transition.duration = MotionSpec.crossfadeDuration
			transition.timingFunction = Tokens.motionCurve
			layer.add(transition, forKey: "motion.crossfade")
		}
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
			if rowContent.isPicked {
				Symbol.set(checkbox, Self.checkboxSymbols[1], size: Self.checkboxSize, color: .controlAccentColor)
			} else {
				Symbol.set(checkbox, Self.checkboxSymbols[0], size: Self.checkboxSize, color: .tertiaryLabelColor)
			}
		}
		for (view, isShown) in [(checkbox, showsCheckbox), (icon, !showsCheckbox), (urgentBadge, !showsCheckbox)] as [(NSView, Bool)] {
			fade(view, to: isShown ? 1 : 0, in: motion)
		}
	}

	private func fade(_ leaf: NSView, to opacity: Float, in motion: MotionChange?) {
		let key = ObjectIdentifier(leaf)
		guard let motion else {
			leaf.layer?.removeAnimation(forKey: ContentMotion.opacityKey)
			leafFades[key] = nil
			leaf.alphaValue = CGFloat(opacity)
			return
		}
		guard leaf.alphaValue != CGFloat(opacity) else { return }
		let shown = leafFades[key]?.opacity(at: motion.time) ?? Float(leaf.alphaValue)
		let spec = MotionSpec.Content(
			fromOpacity: shown, toOpacity: opacity, fromScale: 1, toScale: 1, duration: MotionSpec.crossfadeDuration, delay: 0,
		)
		let fade = ContentMotion(spec: spec, start: motion.time)
		fade.commit(to: leaf)
		leafFades[key] = fade
	}

	/// Where a click picks the row while rows are being picked: the icon's
	/// column, the header's height.
	private var checkboxTarget: NSRect {
		NSRect(x: 0, y: 0, width: RowMetrics.textX - Tokens.columnGap / 2, height: headerHeight)
	}

	/// The minute changed: "4 min ago".
	func refreshTime() {
		context.show(rowContent.message)
	}

	/// Drops the row's motion and its leaves' fades: they show their model.
	override func endMotion() {
		super.endMotion()
		for layer in (headerLeaves + [moreButton].compactMap { $0 }).compactMap(\.layer) {
			layer.removeMotions(keyedFrom: "motion.")
		}
		leafFades = [:]
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
		title.frame = Label.frame(x: RowMetrics.textX, y: top, width: RowMetrics.textWidth, height: titleHeight)
		let contextTop = top + titleHeight + RowMetrics.titleGap
		context.frame = NSRect(x: RowMetrics.textX, y: contextTop, width: RowMetrics.textWidth, height: RowMetrics.contextHeight)
		context.layOut()

		let iconSize = icon.image?.size ?? NSSize(width: 16, height: 16)
		icon.frame = NSRect(
			x: Tokens.rowHorizontalPadding + (Tokens.iconColumn - iconSize.width) / 2, y: top + 1,
			width: iconSize.width, height: iconSize.height,
		)
		// A fixed box, whichever symbol it shows: a pick changes the symbol
		// without laying the row out.
		let box: CGFloat = 20
		checkbox.frame = NSRect(
			x: Tokens.rowHorizontalPadding + (Tokens.iconColumn - box) / 2, y: (icon.frame.midY - box / 2).rounded(),
			width: box, height: box,
		)
		let badgeSize = urgentBadge.image?.size ?? .zero
		urgentBadge.frame = NSRect(
			x: Tokens.rowHorizontalPadding + (Tokens.iconColumn - badgeSize.width) / 2, y: icon.frame.maxY + 3,
			width: badgeSize.width, height: badgeSize.height,
		)
		let iconBottom = urgentBadge.isHidden ? icon.frame.maxY : urgentBadge.frame.maxY

		let markX = Tokens.popoverWidth - Tokens.rowHorizontalPadding - RowMetrics.markWidth
		let markSize = waitingMark.image?.size ?? .zero
		waitingMark.frame = NSRect(
			x: markX + (RowMetrics.markWidth - markSize.width) / 2, y: top + (titleHeight - markSize.height) / 2,
			width: markSize.width, height: markSize.height,
		)
		moreButton?.frame = NSRect(
			x: markX, y: top + (titleHeight - RowMetrics.markWidth) / 2, width: RowMetrics.markWidth, height: RowMetrics.markWidth,
		)
		let markBottom = isExpanded ? waitingMark.frame.maxY : top + max(titleHeight, RowMetrics.markWidth)
		let headerBottom = max(context.frame.maxY, iconBottom, markBottom)
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
	/// moves): its place, closed, is listed at once, so it draws ahead.
	func prepareBody() {
		guard body == nil else { return }
		if moreButton == nil {
			moreButton = makeMoreButton()
		}
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
		if showsMore, moreButton == nil {
			moreButton = makeMoreButton()
			_ = layOutHeader()
		}
		moreButton?.alphaValue = showsMore ? 1 : 0
		waitingMark.alphaValue = showsMore ? 0 : 1
	}

	private func makeMoreButton() -> NSButton {
		let button = MoreButton { [weak self] button in
			guard let self else { return }
			let menu = ItemMenu.make(for: self.rowContent.message, moveTargets: self.rowContent.moveTargets, inbox: self.inbox)
			menu.popUp(positioning: nil, at: NSPoint(x: 0, y: button.bounds.height + 2), in: button)
		}
		// The row's context menu is the same menu for VoiceOver.
		button.setAccessibilityElement(false)
		button.setAccessibilityIdentifier("message-menu:\(rowContent.message.title)")
		button.alphaValue = 0
		headerSlot.content.addSubview(button)
		return button
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
			if let moreButton, moreButton.alphaValue > 0, convert(moreButton.frame, from: moreButton.superview).contains(local) {
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
			context.spokenText,
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

	func show(_ message: InboxMessage) {
		let crumb = InboxWords.breadcrumb(for: message)
		breadcrumb.stringValue = crumb ?? ""
		breadcrumb.isHidden = crumb == nil
		dot.isHidden = crumb == nil
		agentName = message.agentName
		switch InboxWords.agentMark(for: message.agent) {
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
		time.stringValue = InboxWords.timeAgo(message.receivedAt)
		setAccessibilityIdentifier("context:\(message.title)")
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
	private let onPress: (NSButton) -> Void

	init(onPress: @escaping (NSButton) -> Void) {
		self.onPress = onPress
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
		onPress(self)
	}
}
