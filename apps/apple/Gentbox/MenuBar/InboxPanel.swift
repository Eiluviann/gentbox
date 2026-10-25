import AppKit

/// The menu-bar inbox's window: a borderless, non-activating panel under the
/// status item (design: `mac-popover`, a rounded panel without an arrow).
///
/// It must never look like a standard window. Tiling window managers such as
/// AeroSpace tile anything that reports the `AXStandardWindow` subrole, has a
/// title and the usual buttons. This panel has no title or buttons, sits at
/// the pop-up menu level, joins every Space and stays out of the window
/// cycle, so it reports a floating subrole and is left alone.
@MainActor
class InboxPanel: NSPanel {
	/// The panel's settings, which `InboxPanelTests` checks.
	static let styleMask: NSWindow.StyleMask = [.borderless, .nonactivatingPanel]
	static let level: NSWindow.Level = .popUpMenu
	static let collectionBehavior: NSWindow.CollectionBehavior = [
		.canJoinAllSpaces, .transient, .ignoresCycle, .fullScreenAuxiliary,
	]
	static let cornerRadius: CGFloat = 12
	/// Room beside the card for its drawn shadow.
	static let margin: CGFloat = 24
	/// Room above the card: no more than the gap under the menu bar, whose
	/// items must keep their clicks while the inbox shows. The shadow's
	/// faint top is cut there.
	static let topRoom: CGFloat = gap
	/// Room below the card for its shadow, which falls 6 points down.
	static let bottomRoom: CGFloat = shadowOffset + shadowPadding

	/// The window hugs the card and its shadow, so the window server gives
	/// the card every click by itself, with no pointer tracking, and clicks
	/// below the shadow reach the windows under it. Its top never moves. It
	/// is resized at most twice per motion, never each frame: grown before a
	/// motion that grows the card, shrunk after one that shrinks it. Resizing
	/// a window each frame makes AppKit lay out its whole view tree and the
	/// window server rebuild its surface; moving a few layers costs neither.
	///
	/// The card is three layers the render server draws cheaply as they
	/// move: the surface, card-sized and rounded by its own corners (a mask
	/// layer would be an offscreen pass each frame), a hairline ring, and a
	/// shadow drawn once and stretched (a live shadow would be blurred again
	/// each frame). The content inside the surface is laid out for the
	/// tallest card and hangs from the top, so the card's height never lays
	/// it out.
	private let stage = StageView()
	private let surface = SurfaceView()
	private let ring = CALayer()
	private let shadow = CALayer()
	/// The shadow under the surface and the ring over it, each in a view of
	/// its own: AppKit orders a view's layer among its siblings' by the
	/// views, and would sort layers added to the stage's own under the
	/// surface.
	private let shadowHost = LayerHostView()
	private let ringHost = LayerHostView()
	/// What the card shows: `InboxContentView`.
	let content: NSView
	/// The card's height as drawn now.
	private(set) var cardHeight: CGFloat = 0

	init(content: NSView) {
		self.content = content
		super.init(contentRect: .zero, styleMask: Self.styleMask, backing: .buffered, defer: true)
		// Before `level`: setting it resets the level to `.floating`.
		isFloatingPanel = true
		level = Self.level
		collectionBehavior = Self.collectionBehavior
		hidesOnDeactivate = false
		isMovable = false
		isReleasedWhenClosed = false
		// The card draws its own shadow, which follows it as it grows and as
		// it spawns.
		hasShadow = false
		isOpaque = false
		backgroundColor = .clear
		animationBehavior = .none
		setAccessibilityIdentifier("gentbox.inbox")
		setAccessibilityTitle("Gentbox Inbox")
		buildCard(content: content)
		contentView = stage
	}

	/// The card's height: the surface, the ring and the shadow move
	/// together, with no view layout. Animated, they move in the transaction
	/// that moves the content (`InboxContentView`), on its curve, from where
	/// they show.
	func setCardHeight(_ height: CGFloat, animated: Bool = false) {
		cardHeight = height
		let card = NSRect(x: Self.margin, y: Self.topRoom, width: Tokens.popoverWidth, height: height)
		var motion = LayerMotion()
		motion.capture(surface.layer, ["position", "bounds"])
		motion.capture(ring, ["position", "bounds"])
		motion.capture(shadow, ["position", "bounds"])
		CATransaction.begin()
		CATransaction.setDisableActions(true)
		surface.frame = card
		ring.frame = card
		shadow.frame = card.offsetBy(dx: 0, dy: Self.shadowOffset).insetBy(dx: -Self.shadowPadding, dy: -Self.shadowPadding)
		// Grown first, so the card never shows cut; shrunk only once it has.
		fitWindow(cardHeight: animated ? max(height, windowCardHeight) : height)
		motion.commit(animated: animated)
		CATransaction.commit()
		// Clicks reach the card as it will be: the motion is a quarter second.
		stage.card = card
		windowFitGeneration += 1
		if animated, windowCardHeight > height {
			let generation = windowFitGeneration
			DispatchQueue.main.asyncAfter(deadline: .now() + Tokens.motionDuration) { [weak self] in
				MainActor.assumeIsolated {
					guard let self, self.windowFitGeneration == generation else { return }
					self.fitWindow(cardHeight: self.cardHeight)
				}
			}
		}
	}

	// MARK: The window around the card

	/// Each shrink waits for the motion before it; a later change cancels it.
	private var windowFitGeneration = 0

	/// The card height the window has room for now.
	private var windowCardHeight: CGFloat {
		frame.height - Self.topRoom - Self.bottomRoom
	}

	/// Resizes the window to a card of `height`, its top held.
	private func fitWindow(cardHeight height: CGFloat) {
		let fitted = Self.windowFrame(card: NSRect(x: frame.minX + Self.margin, y: frame.maxY - Self.topRoom - height, width: Tokens.popoverWidth, height: height))
		if fitted != frame {
			setFrame(fitted, display: false, animate: false)
		}
	}

	/// The card's frame on screen, where it settles.
	var cardFrameOnScreen: NSRect {
		cardFrame(height: cardHeight)
	}

	/// The card's frame on screen as drawn now, mid-motion too.
	var presentedCardFrameOnScreen: NSRect {
		cardFrame(height: (surface.layer?.presentation() ?? surface.layer)?.bounds.height ?? cardHeight)
	}

	private func cardFrame(height: CGFloat) -> NSRect {
		NSRect(x: frame.minX + Self.margin, y: frame.maxY - Self.topRoom - height, width: Tokens.popoverWidth, height: height)
	}

	private func buildCard(content: NSView) {
		stage.wantsLayer = true
		stage.layer?.masksToBounds = false
		shadow.contentsGravity = .resize
		shadowHost.hostedLayer.addSublayer(shadow)
		stage.addSubview(shadowHost)

		// The design's white surface, not the system's vibrant material, which
		// tints it grey (an.css:193), rounded by its own corners.
		surface.wantsLayer = true
		surface.layer?.cornerRadius = Self.cornerRadius
		surface.layer?.cornerCurve = .continuous
		surface.layer?.masksToBounds = true
		stage.addSubview(surface)
		surface.addSubview(content)

		ring.cornerRadius = Self.cornerRadius
		ring.cornerCurve = .continuous
		ring.borderWidth = 0.5
		ringHost.hostedLayer.addSublayer(ring)
		stage.addSubview(ringHost)
		stage.onAppearanceChange = { [weak self] in self?.paint() }
		stage.onClickBesideCard = { [weak self] in self?.onClose?() }
	}

	/// The surface and ring colours in the panel's appearance, and the
	/// shadow at its backing scale.
	private func paint() {
		effectiveAppearance.performAsCurrentDrawingAppearance {
			surface.layer?.backgroundColor = Tokens.surface.cgColor
			ring.borderColor = Tokens.surfaceOutline.cgColor
		}
		let scale = backingScaleFactor
		guard shadow.contentsScale != scale || shadow.contents == nil else { return }
		let image = Self.shadowImage(scale: scale)
		let side = CGFloat(image.width)
		let middle = Self.shadowSlice * scale
		shadow.contents = image
		shadow.contentsScale = scale
		shadow.contentsCenter = CGRect(x: (side - middle) / 2 / side, y: (side - middle) / 2 / side, width: middle / side, height: middle / side)
	}

	/// The card's shadow, `--shadow-window` (an.css:35): black at 22 %,
	/// Core Animation's `shadowRadius` 14, 6 points down.
	private static let shadowOffset: CGFloat = 6
	/// Core Graphics' blur for `shadowRadius` 14: 2.1 times it, fitted to
	/// the shadow Core Animation drew on screen.
	private static let shadowBlur: CGFloat = 14 * 2.1
	/// How far the blur reaches past the card's edge, and in from it.
	private static let shadowPadding: CGFloat = (shadowBlur * 1.5).rounded(.up)
	/// The stretched middle of the nine-slice image.
	private static let shadowSlice: CGFloat = 2

	/// A rounded card's shadow, blurred once: corners and edges in its nine
	/// slices, a middle that stretches to any card. The card in the image
	/// reaches past the blur on every side, so the middle is at full
	/// strength.
	private static func shadowImage(scale: CGFloat) -> CGImage {
		let card = 2 * shadowPadding + shadowSlice
		let side = Int(((card + 2 * shadowPadding) * scale).rounded())
		let context = CGContext(
			data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
			space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(),
			bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue,
		)
		guard let context else { fatalError("a bitmap context of \(side) pixels") }
		// The card itself is drawn far to the left, so only its shadow, offset
		// back into the image, lands: the surface is not quite opaque.
		let away = CGFloat(side) * 4
		context.setShadow(
			offset: CGSize(width: away, height: 0), blur: shadowBlur * scale,
			color: NSColor.black.withAlphaComponent(0.22).cgColor,
		)
		let rect = CGRect(x: shadowPadding * scale - away, y: shadowPadding * scale, width: card * scale, height: card * scale)
		context.addPath(CGPath(roundedRect: rect, cornerWidth: cornerRadius * scale, cornerHeight: cornerRadius * scale, transform: nil))
		context.fillPath()
		guard let image = context.makeImage() else { fatalError("a shadow image") }
		return image
	}

	/// Hangs the window around `card` and lays the content out for the
	/// tallest card, `maxCardHeight`; the card's height is set apart.
	func place(card: NSRect, maxCardHeight: CGFloat) {
		// A frame set through a scaled layer lands off its place: the last
		// despawn left the stage at 0.98.
		resetStage(opacity: 0)
		setFrame(Self.windowFrame(card: card), display: false, animate: false)
		content.frame = NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: maxCardHeight)
		for host in [shadowHost, ringHost] {
			host.frame = stage.bounds
			host.autoresizingMask = [.width, .height]
		}
		paint()
		setCardHeight(card.height)
	}

	// MARK: Spawn and despawn

	/// Whether it is fading out, ignoring input, before it is ordered out.
	private(set) var isDespawning = false
	/// Each despawn's completion runs only if no spawn came after it.
	private var despawnGeneration = 0

	/// Fades and scales in from the card's top centre (fade only with Reduce
	/// Motion), from where a despawn in progress has got to, so a re-open
	/// reverses it without a jump. Input works at once.
	func spawn(reduceMotion: Bool) {
		guard let layer = stage.layer else { return }
		despawnGeneration += 1
		isDespawning = false
		ignoresMouseEvents = false
		let presentation = layer.presentation()
		let fromOpacity = isVisible ? (presentation?.opacity ?? layer.opacity) : 0
		let fromScale = isVisible ? Self.scale(of: presentation?.transform ?? layer.transform) : (reduceMotion ? 1 : 0.96)
		animate(layer, opacity: (fromOpacity, 1), scale: (fromScale, 1), duration: 0.18, timing: .easeOut, completion: nil)
	}

	/// Fades and scales out (fade only with Reduce Motion), ignoring input,
	/// and calls `completion` once it has finished, unless a spawn came first.
	func despawn(reduceMotion: Bool, completion: @escaping () -> Void) {
		guard let layer = stage.layer else { return completion() }
		isDespawning = true
		ignoresMouseEvents = true
		despawnGeneration += 1
		let generation = despawnGeneration
		let presentation = layer.presentation()
		animate(
			layer, opacity: (presentation?.opacity ?? layer.opacity, 0),
			scale: (Self.scale(of: presentation?.transform ?? layer.transform), reduceMotion ? 1 : 0.98),
			duration: 0.12, timing: .easeIn,
		) { [weak self] in
			guard let self, self.despawnGeneration == generation else { return }
			self.isDespawning = false
			completion()
			self.resetStage(opacity: 0)
		}
	}

	/// The opacity and scale on screen now, for the trace and the tests.
	var presentedSpawn: (opacity: Float, scale: CGFloat) {
		guard let layer = stage.layer else { return (0, 0) }
		let presented = layer.presentation() ?? layer
		return (isVisible ? presented.opacity : 0, Self.scale(of: presented.transform))
	}

	/// Unscaled and at `opacity`, with no animation left on it.
	private func resetStage(opacity: Float) {
		guard let layer = stage.layer else { return }
		CATransaction.begin()
		CATransaction.setDisableActions(true)
		layer.removeAnimation(forKey: "spawn")
		layer.transform = CATransform3DIdentity
		layer.opacity = opacity
		CATransaction.commit()
	}

	/// The scale and fade in one Core Animation transaction: the render
	/// server runs it, no frame of it touches the main thread.
	private func animate(
		_ layer: CALayer, opacity: (Float, Float), scale: (CGFloat, CGFloat), duration: CFTimeInterval,
		timing: CAMediaTimingFunctionName, completion: (() -> Void)?,
	) {
		let fade = CABasicAnimation(keyPath: "opacity")
		fade.fromValue = opacity.0
		fade.toValue = opacity.1
		let grow = CABasicAnimation(keyPath: "transform")
		grow.fromValue = NSValue(caTransform3D: cardTopTransform(scale: scale.0))
		grow.toValue = NSValue(caTransform3D: cardTopTransform(scale: scale.1))
		let group = CAAnimationGroup()
		group.animations = [fade, grow]
		group.duration = duration
		group.timingFunction = CAMediaTimingFunction(name: timing)
		CATransaction.begin()
		CATransaction.setCompletionBlock { completion?() }
		CATransaction.setDisableActions(true)
		layer.opacity = opacity.1
		layer.transform = cardTopTransform(scale: scale.1)
		layer.add(group, forKey: "spawn")
		CATransaction.commit()
	}

	/// Scales about the card's top centre, so the card never travels. A
	/// view's layer is anchored at its origin, in its superlayer's y-up space.
	func cardTopTransform(scale: CGFloat) -> CATransform3D {
		let cardTop = CGPoint(x: frame.width / 2, y: frame.height - Self.topRoom)
		return CATransform3DMakeAffineTransform(
			CGAffineTransform(
				a: scale, b: 0, c: 0, d: scale,
				tx: (1 - scale) * cardTop.x, ty: (1 - scale) * cardTop.y,
			),
		)
	}

	private static func scale(of transform: CATransform3D) -> CGFloat {
		transform.m11
	}

	/// The window for a card: the card with room for its shadow.
	static func windowFrame(card: NSRect) -> NSRect {
		NSRect(
			x: card.minX - margin, y: card.minY - bottomRoom,
			width: card.width + 2 * margin, height: card.height + topRoom + bottomRoom,
		)
	}

	/// It takes the keyboard (the card's ↩, ⌘⌫ and R, the reply field)
	/// without activating the app.
	override var canBecomeKey: Bool { true }
	override var canBecomeMain: Bool { false }


	/// How the panel closes: the controller's close, which also removes its
	/// click monitor and resets the inbox's expansion, filter and selection.
	var onClose: (() -> Void)?

	/// Escape closes it, as it would a menu.
	override func cancelOperation(_: Any?) {
		onClose?()
	}

	/// A key nothing focused took: the card's buttons answer to it (R has no
	/// modifier, which AppKit leaves to the views otherwise).
	override func keyDown(with event: NSEvent) {
		if !performKeyEquivalent(with: event) {
			super.keyDown(with: event)
		}
	}

	/// Only an open reply box holds focus. On macOS 15 a window that
	/// becomes key focuses the first view in its key loop that takes it, and
	/// a focused text field would take the card's R and ↩ as text.
	override func becomeKey() {
		super.becomeKey()
		if let editor = firstResponder as? NSTextView, editor.isFieldEditor,
		   let field = editor.delegate as? NSTextField, !field.isEditable || field.alphaValue == 0
		{
			makeFirstResponder(nil)
		}
	}

	/// Losing the keyboard to another app (⌘Tab, a click elsewhere) closes
	/// it, so it never floats over every Space unattended.
	override func resignKey() {
		super.resignKey()
		onClose?()
	}

	/// Where the panel goes: under `anchor` (the status item, in screen
	/// coordinates), centred on it, kept inside the screen's visible frame,
	/// and no taller than `maxHeight(under:in:)`. With the menu bar hidden
	/// the item sits above the screen; the panel then hangs from the top of
	/// the visible frame. An anchor that is not at the top of the screen (the
	/// item's window before the menu bar is laid out reports (0, 0) or a y
	/// below the screen) counts as none, and the panel hangs at the top right,
	/// where the item would be.
	static func frame(size: NSSize, under anchor: NSRect, in visibleFrame: NSRect, gap: CGFloat = gap) -> NSRect {
		let anchor = menuBarAnchor(anchor, in: visibleFrame)
		let top = top(under: anchor, in: visibleFrame, gap: gap)
		var x = anchor.midX - size.width / 2
		x = min(max(x, visibleFrame.minX + gap), visibleFrame.maxX - size.width - gap)
		let height = min(size.height, top - visibleFrame.minY - gap)
		return NSRect(x: x, y: top - height, width: size.width, height: height)
	}

	/// The tallest the panel gets: from its top down to the bottom of the
	/// visible frame. Only content taller than this scrolls.
	static func maxHeight(under anchor: NSRect, in visibleFrame: NSRect, gap: CGFloat = gap) -> CGFloat {
		top(under: menuBarAnchor(anchor, in: visibleFrame), in: visibleFrame, gap: gap) - visibleFrame.minY - gap
	}

	static let gap: CGFloat = 4

	private static func menuBarAnchor(_ anchor: NSRect, in visibleFrame: NSRect) -> NSRect {
		let isInMenuBar = anchor.width > 0 && anchor.height > 0 && anchor.minY >= visibleFrame.maxY - 1
		return isInMenuBar ? anchor : NSRect(x: visibleFrame.maxX, y: visibleFrame.maxY, width: 0, height: 0)
	}

	private static func top(under anchor: NSRect, in visibleFrame: NSRect, gap: CGFloat) -> CGFloat {
		min(anchor.minY - gap, visibleFrame.maxY - gap)
	}
}

/// The panel's content view: flipped, so the card hangs from the top, and
/// transparent around the card, where its shadow falls. A click there is a
/// click beside the card, which closes the panel as one anywhere else does.
final class StageView: NSView {
	var card: NSRect = .zero
	var onAppearanceChange: (() -> Void)?
	var onClickBesideCard: (() -> Void)?

	override var isFlipped: Bool { true }

	override func hitTest(_ point: NSPoint) -> NSView? {
		guard bounds.contains(convert(point, from: superview)) else { return nil }
		return card.contains(convert(point, from: superview)) ? super.hitTest(point) : self
	}

	override func mouseDown(with _: NSEvent) {
		onClickBesideCard?()
	}

	override func rightMouseDown(with _: NSEvent) {
		onClickBesideCard?()
	}

	override func viewDidChangeEffectiveAppearance() {
		super.viewDidChangeEffectiveAppearance()
		onAppearanceChange?()
	}
}

/// A view that hosts layers this panel draws itself, ordered among the
/// stage's views, flipped like the stage, and never hit by a click.
@MainActor
private final class LayerHostView: NSView {
	let hostedLayer = CALayer()

	init() {
		super.init(frame: .zero)
		hostedLayer.isGeometryFlipped = true
		layer = hostedLayer
		wantsLayer = true
		autoresizingMask = [.width, .height]
	}

	@available(*, unavailable)
	required init?(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	override var isFlipped: Bool { true }

	override func hitTest(_: NSPoint) -> NSView? {
		nil
	}
}

#if DEBUG || GENTBOX_HOOKS
/// The panel for recordings (`DemoDriver`): it draws as when opened from
/// the status item, which makes it key, though the harness never takes the
/// keyboard. AppKit draws the accent bezel only with a key appearance, so
/// these are its own queries, answered here; no build you run day to day
/// defines either flag.
@MainActor
final class RecordingInboxPanel: InboxPanel {
	@objc var hasKeyAppearance: Bool { true }
	@objc func _hasKeyAppearance() -> Bool { true }
	@objc func _hasActiveAppearance() -> Bool { true }
}
#endif

/// The card's surface: content hangs from its top while its height moves.
@MainActor
private final class SurfaceView: NSView {
	override var isFlipped: Bool { true }
}
