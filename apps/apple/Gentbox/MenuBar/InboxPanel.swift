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
final class InboxPanel: NSPanel {
	/// The panel's settings, which `InboxPanelTests` checks.
	static let styleMask: NSWindow.StyleMask = [.borderless, .nonactivatingPanel]
	static let level: NSWindow.Level = .popUpMenu
	static let collectionBehavior: NSWindow.CollectionBehavior = [
		.canJoinAllSpaces, .transient, .ignoresCycle, .fullScreenAuxiliary,
	]
	static let cornerRadius: CGFloat = 12

	init(content: NSView) {
		super.init(contentRect: .zero, styleMask: Self.styleMask, backing: .buffered, defer: true)
		// Before `level`: setting it resets the level to `.floating`.
		isFloatingPanel = true
		level = Self.level
		collectionBehavior = Self.collectionBehavior
		hidesOnDeactivate = false
		isMovable = false
		isReleasedWhenClosed = false
		hasShadow = true
		isOpaque = false
		backgroundColor = .clear
		animationBehavior = .utilityWindow
		setAccessibilityIdentifier("gentbox.inbox")
		setAccessibilityTitle("Gentbox Inbox")

		// The design's white surface, not the system's vibrant material, which
		// tints it grey (an.css:193); rounded, with the window shadow's ring.
		let background = SurfaceView()
		background.wantsLayer = true
		background.layer?.cornerRadius = Self.cornerRadius
		background.layer?.cornerCurve = .continuous
		background.layer?.masksToBounds = true
		background.layer?.borderWidth = 0.5
		content.translatesAutoresizingMaskIntoConstraints = false
		background.addSubview(content)
		NSLayoutConstraint.activate([
			content.leadingAnchor.constraint(equalTo: background.leadingAnchor),
			content.trailingAnchor.constraint(equalTo: background.trailingAnchor),
			content.topAnchor.constraint(equalTo: background.topAnchor),
			content.bottomAnchor.constraint(equalTo: background.bottomAnchor),
		])
		contentView = background
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

	/// Losing the keyboard to another app (⌘Tab, a click elsewhere) closes
	/// it, so it never floats over every Space unattended.
	override func resignKey() {
		super.resignKey()
		onClose?()
	}

	/// Where the panel goes: under `anchor` (the status item, in screen
	/// coordinates), centred on it, kept inside the screen's visible frame.
	/// With the menu bar hidden the item sits above the screen; the panel
	/// then hangs from the top of the visible frame. An anchor that is not at
	/// the top of the screen (the item's window before the menu bar is laid
	/// out reports (0, 0) or a y below the screen) counts as none, and the
	/// panel hangs at the top right.
	static func frame(size: NSSize, under anchor: NSRect, in visibleFrame: NSRect, gap: CGFloat = 4) -> NSRect {
		let isInMenuBar = anchor.width > 0 && anchor.height > 0 && anchor.minY >= visibleFrame.maxY - 1
		let anchor = isInMenuBar ? anchor : NSRect(x: visibleFrame.maxX, y: visibleFrame.maxY, width: 0, height: 0)
		let top = min(anchor.minY - gap, visibleFrame.maxY - gap)
		var x = anchor.midX - size.width / 2
		x = min(max(x, visibleFrame.minX + gap), visibleFrame.maxX - size.width - gap)
		let height = min(size.height, top - visibleFrame.minY - gap)
		return NSRect(x: x, y: top - height, width: size.width, height: height)
	}
}

/// Paints the inbox's surface and ring in the current appearance.
private final class SurfaceView: NSView {
	override var wantsUpdateLayer: Bool { true }

	override func updateLayer() {
		layer?.backgroundColor = Tokens.surface.cgColor
		layer?.borderColor = Tokens.surfaceOutline.cgColor
	}
}
