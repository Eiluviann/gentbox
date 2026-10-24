import AppKit
import GentboxCore

/// Everything a row draws: a row changes only when this does.
struct RowContent: Equatable {
	let message: InboxMessage
	let isExpanded: Bool
	let isPicked: Bool
	/// The first item draws no rule above it.
	let isFirst: Bool
	/// The reply field's draft, when it is on this message.
	let replyDraft: InboxModel.ReplyDraft?
	let moveTargets: [TopicChoice]
}

/// One item of the accordion (design: `mac-popover`: "the active card keeps
/// a row's order and adds its body below, so a row can grow into it").
/// Collapsed, it is a two-line row: type icon, title, breadcrumb, and the
/// waiting mark, which the ellipsis menu replaces on hover. Expanded, the
/// same header takes more padding and the body and buttons are revealed
/// below it, inside the row's own clipped bounds, which Core Animation
/// grows: one view throughout, so nothing cross-fades. Its text keeps the
/// row's sizes (the user's decision): expanding adds content and actions and
/// enlarges nothing. Only the expanded item has buttons, so ↩, ⌘⌫ and R act
/// on it alone.
///
/// For VoiceOver a collapsed item is one element: its title, breadcrumb,
/// agent and waiting mark in one label, "Muted" as its value, and a press
/// that expands it. The card contains its buttons and field.
///
/// A click on a collapsed item's header expands it (and collapses the open
/// one); a click on the expanded item's header collapses it; ⌘-click picks a
/// collapsed item for a bulk Dismiss (never an `action_required` one).
@MainActor
final class InboxRowView: FlippedView {
	/// The message it shows; a message that supersedes it takes the row over.
	private(set) var messageId: String
	private(set) var content: RowContent
	private let inbox: InboxModel
	private let rule = CALayer()
	private let icon = NSImageView()
	private let urgentBadge = Symbol.view("figure.walk.motion", size: 12, color: Tokens.waiting)
	private let title = Label.line(size: 13, weight: .semibold)
	private let context = ContextLine()
	private let waitingMark = NSImageView()
	/// Built ahead with the body and kept drawn at zero opacity until a
	/// hover; it pops up the item menu, built only then.
	private var moreButton: NSButton?
	/// Built ahead (`prepareBody`) and kept while collapsed, transparent and
	/// clipped away, so expanding only reveals it.
	private var body: CardBodyView?
	private var isHovered = false {
		didSet { if isHovered != oldValue { showHover() } }
	}

	private var isPressInHeader = false
	private var hasShownMessage = false
	/// Fading out after it left the list, or parked: no clicks or keys.
	var isLeaving = false {
		didSet {
			// Leaving or parked, its card takes no clicks, keys or VoiceOver.
			body?.isActive = !isLeaving && content.isExpanded
		}
	}
	private(set) var headerHeight: CGFloat = 0

	init(content: RowContent, inbox: InboxModel) {
		messageId = content.message.id
		self.content = content
		self.inbox = inbox
		super.init(frame: NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: 0))
		layer?.masksToBounds = true
		rule.anchorPoint = .zero
		layer?.addSublayer(rule)
		icon.imageScaling = .scaleNone
		// Decorative: `checkmark.circle` would read "Selected" and carry the
		// Selected trait, which bulk Dismiss's picks use.
		icon.setAccessibilityElement(false)
		urgentBadge.setAccessibilityLabel("Urgent")
		waitingMark.imageScaling = .scaleNone
		for view in [icon, urgentBadge, title, context, waitingMark] as [NSView] {
			view.wantsLayer = true
			addSubview(view)
		}
		addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
		apply(content, isAnimated: false)
	}

	/// The row's height for its content.
	var height: CGFloat {
		frame.height
	}

	/// Notes where its moving parts show now, before a change.
	func capture(into motion: inout LayerMotion) {
		motion.capture(layer, ["position", "bounds"])
		for view in [icon, urgentBadge, title, context, waitingMark, moreButton, body] as [NSView?] {
			motion.capture(view?.layer, ["position"])
		}
		body?.capture(into: &motion)
	}

	/// Shows `content`, laying out its parts; the caller places the row and
	/// commits the motion.
	func apply(_ newContent: RowContent, isAnimated: Bool) {
		let wasExpanded = content.isExpanded
		let isNewMessage = !hasShownMessage || newContent.message != content.message
		content = newContent
		messageId = newContent.message.id
		let message = newContent.message
		if isNewMessage {
			show(message)
		}
		fill = background
		if newContent.isExpanded, body == nil {
			makeBody()
		}
		body?.show(message: message, replyDraft: newContent.replyDraft)
		if newContent.isExpanded, !wasExpanded || isAnimated == false {
			showBody(isAnimated: isAnimated && !wasExpanded)
		} else if !newContent.isExpanded, wasExpanded {
			hideBody(isAnimated: isAnimated)
		}
		if !newContent.isExpanded, isHovered {
			showHover()
		}
		moreButton?.alphaValue = newContent.isExpanded || !isHovered ? 0 : 1
		waitingMark.alphaValue = isHovered && !newContent.isExpanded ? 0 : 1
		layOut()
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

	/// The minute changed: "4 min ago".
	func refreshTime() {
		context.show(content.message)
	}

	// MARK: Layout

	private func layOut() {
		let isExpanded = content.isExpanded
		let ruleHeight: CGFloat = content.isFirst ? 0 : 1
		let top = ruleHeight + (isExpanded ? 14 : 9)
		let titleHeight = RowMetrics.titleLineHeight
		title.frame = Label.frame(x: RowMetrics.textX, y: top, width: RowMetrics.textWidth, height: titleHeight)
		let contextTop = top + titleHeight + (isExpanded ? 2 : 1)
		context.frame = NSRect(x: RowMetrics.textX, y: contextTop, width: RowMetrics.textWidth, height: RowMetrics.contextHeight)
		context.layOut()

		let iconSize = icon.image?.size ?? NSSize(width: 16, height: 16)
		icon.frame = NSRect(
			x: Tokens.rowHorizontalPadding + (Tokens.iconColumn - iconSize.width) / 2, y: top + 1,
			width: iconSize.width, height: iconSize.height,
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
		headerHeight = headerBottom
		var height = headerBottom
		if let body {
			let bodyHeight = body.layOut(width: RowMetrics.bodyWidth)
			body.frame = NSRect(x: RowMetrics.textX, y: headerBottom, width: RowMetrics.bodyWidth, height: bodyHeight)
			if isExpanded {
				height += bodyHeight
			}
		}
		height += isExpanded ? 16 : 9
		// Up to the device pixel, as SwiftUI laid rows out.
		let pixels = window?.backingScaleFactor ?? 2
		frame.size = NSSize(width: Tokens.popoverWidth, height: (height * pixels).rounded(.up) / pixels)
		CATransaction.begin()
		CATransaction.setDisableActions(true)
		rule.isHidden = content.isFirst
		rule.frame = NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: 1)
		CATransaction.commit()
	}

	override func updateLayer() {
		super.updateLayer()
		rule.backgroundColor = Tokens.line.cgColor
	}

	private var background: NSColor {
		if content.isPicked { return RowMetrics.pickedFill }
		return isHovered && !content.isExpanded ? RowMetrics.hoverFill : Tokens.row
	}

	// MARK: The card's body

	/// Builds the card's body before anyone expands the row, its controls a
	/// piece at a time (`IdleWork`).
	func prepareBody() {
		guard body == nil else { return }
		if moreButton == nil {
			moreButton = makeMoreButton()
		}
		makeBody()
		body?.show(message: content.message, replyDraft: content.replyDraft)
		layOut()
	}

	var hasBody: Bool {
		body != nil
	}

	/// Expanded: the body shows below the header. Revealed by the row's
	/// growth, it fades in over the second half of the motion, so it never
	/// draws over a moving header.
	private func showBody(isAnimated: Bool) {
		guard let body else { return }
		body.finishControls()
		body.isActive = true
		if isAnimated {
			let half = Tokens.motionDuration / 2
			let isFadingOut = body.layer?.animation(forKey: "motion.opacity") != nil
			LayerMotion.fade(body.layer, to: 1, duration: half, delay: isFadingOut ? 0 : half, from: isFadingOut ? nil : 0)
		} else {
			body.layer?.removeAnimation(forKey: "motion.opacity")
			body.layer?.opacity = 1
		}
	}

	/// Collapsed: the body fades out at once, before the rows slide up, and
	/// stays, clipped away, for the next expand.
	private func hideBody(isAnimated: Bool) {
		guard let body else { return }
		body.isActive = false
		if isAnimated {
			LayerMotion.fade(body.layer, to: 0, duration: 0.08)
		} else {
			body.layer?.removeAnimation(forKey: "motion.opacity")
			body.layer?.opacity = 0
		}
	}

	private func makeBody() {
		let body = CardBodyView(inbox: inbox)
		body.layer?.opacity = 0
		addSubview(body)
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
		fill = background
		let showsMore = isHovered && !content.isExpanded
		if showsMore, moreButton == nil {
			moreButton = makeMoreButton()
			layOut()
		}
		moreButton?.alphaValue = showsMore ? 1 : 0
		waitingMark.alphaValue = showsMore ? 0 : 1
	}

	private func makeMoreButton() -> NSButton {
		let button = MoreButton { [weak self] button in
			guard let self else { return }
			let menu = ItemMenu.make(for: self.content.message, moveTargets: self.content.moveTargets, inbox: self.inbox)
			menu.popUp(positioning: nil, at: NSPoint(x: 0, y: button.bounds.height + 2), in: button)
		}
		// The row's context menu is the same menu for VoiceOver.
		button.setAccessibilityElement(false)
		button.setAccessibilityIdentifier("message-menu:\(content.message.title)")
		button.alphaValue = 0
		addSubview(button)
		return button
	}

	override func menu(for _: NSEvent) -> NSMenu? {
		ItemMenu.make(for: content.message, moveTargets: content.moveTargets, inbox: inbox)
	}

	override func acceptsFirstMouse(for _: NSEvent?) -> Bool {
		true
	}

	/// A row on its way out, or parked while the hub confirms an answer,
	/// keeps its card's controls but answers no keys: ↩ belongs to the card
	/// that shows.
	override func performKeyEquivalent(with event: NSEvent) -> Bool {
		guard !isLeaving, content.isExpanded else { return false }
		return super.performKeyEquivalent(with: event)
	}

	/// The header is one target: its labels and icons take no clicks.
	override func hitTest(_ point: NSPoint) -> NSView? {
		guard let superview, !isHidden, !isLeaving else { return nil }
		let local = convert(point, from: superview)
		guard bounds.contains(local) else { return nil }
		if local.y < headerHeight {
			if let moreButton, moreButton.alphaValue > 0, moreButton.frame.contains(local) {
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
		if !content.isExpanded, event.modifierFlags.contains(.command) {
			Interactions.untilNextFrame("Select")
			inbox.toggleSelection(of: content.message)
		} else {
			Interactions.untilNextFrame("Expand")
			inbox.toggleExpansion(of: messageId)
		}
	}

	// MARK: Accessibility

	private func describeForAccessibility() {
		let message = content.message
		setAccessibilityElement(true)
		setAccessibilityRole(content.isExpanded ? .group : .button)
		setAccessibilityIdentifier(content.isExpanded ? "card:\(message.title)" : "message:\(message.title)")
		// An expanded card's title is read by its own label inside it.
		setAccessibilityLabel(content.isExpanded ? nil : collapsedLabel)
		setAccessibilityValue(InboxWords.isMuted(message) ? "Muted" : "")
		setAccessibilitySelected(content.isPicked)
		// ⌘-click picks a row for bulk Dismiss; VoiceOver picks it here.
		let canPick = !content.isExpanded && inbox.canSelect(message)
		setAccessibilityCustomActions(canPick ? [
			NSAccessibilityCustomAction(name: content.isPicked ? "Unpick" : "Pick for Dismiss") { [weak self] in
				MainActor.assumeIsolated {
					guard let self else { return false }
					self.inbox.toggleSelection(of: self.content.message)
					return true
				}
			},
		] : [])
	}

	/// VoiceOver's "show menu": the item menu the ellipsis and a right-click
	/// open.
	override func accessibilityPerformShowMenu() -> Bool {
		let menu = ItemMenu.make(for: content.message, moveTargets: content.moveTargets, inbox: inbox)
		menu.popUp(positioning: nil, at: NSPoint(x: RowMetrics.textX, y: headerHeight), in: self)
		return true
	}

	/// Title, breadcrumb, agent, time and waiting mark, as VoiceOver reads a
	/// combined row.
	private var collapsedLabel: String {
		let message = content.message
		return [
			message.isUrgent ? "Urgent" : nil,
			message.title,
			context.spokenText,
			InboxWords.waitingLabel(for: message.blocking),
		].compactMap { $0 }.joined(separator: ", ")
	}

	override func accessibilityChildren() -> [Any]? {
		content.isExpanded ? super.accessibilityChildren() : []
	}

	override func accessibilityPerformPress() -> Bool {
		inbox.toggleExpansion(of: messageId)
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
