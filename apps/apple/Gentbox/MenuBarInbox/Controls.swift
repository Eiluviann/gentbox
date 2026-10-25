import AppKit

/// Every button is the system's own (the user's decision), so its shape and
/// colour follow the OS on macOS 15 and 26: the accent-filled push button
/// for the primary action, the plain push button for the others, a
/// borderless link for text actions; destructive ones (Deny) say so to the
/// system, which draws them plain in a panel. Native control sizes replace the design's paddings: the
/// boards' 28-point `.btn` is `.large`, the 22-point `.btn.sm` `.regular`.
enum ButtonSize {
	case regular
	case small

	var controlSize: NSControl.ControlSize {
		self == .regular ? .large : .regular
	}
}

enum ButtonRole {
	/// Accent-filled. With ↩ it is the window's default button.
	case primary
	case secondary
	/// Deny: marked destructive, drawn as the system draws it.
	case destructive
	/// Text in the accent, no bezel: Show all.
	case link
}

/// A button that runs a closure, takes the first click in the panel (which
/// never activates the app), and shows its keyboard hint (↩, ⌘⌫, R) after
/// the title, smaller (and at 60 % on the plain bezel). VoiceOver reads the
/// title alone.
@MainActor
final class InboxButton: NSButton {
	private let onPress: () -> Void
	private let role: ButtonRole

	init(title: String, hint: String? = nil, role: ButtonRole, size: ButtonSize = .regular, onPress: @escaping () -> Void) {
		self.onPress = onPress
		self.role = role
		super.init(frame: .zero)
		target = self
		action = #selector(press)
		setButtonType(.momentaryPushIn)
		if role == .link {
			isBordered = false
			attributedTitle = NSAttributedString(
				string: title, attributes: [.font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.linkColor],
			)
		} else {
			bezelStyle = .push
			controlSize = size.controlSize
			updateBezel()
			hasDestructiveAction = role == .destructive
			attributedTitle = Self.title(title, hint: hint, role: role, size: size)
		}
		setAccessibilityLabel(title)
		sizeToFit()
	}

	@available(*, unavailable)
	required init?(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	override func acceptsFirstMouse(for _: NSEvent?) -> Bool {
		true
	}

	override var keyEquivalent: String {
		didSet { updateBezel() }
	}

	/// A primary with ↩ is the default button, which AppKit draws in the
	/// accent itself; any other primary takes the accent as its bezel.
	private func updateBezel() {
		guard role == .primary else { return }
		bezelColor = keyEquivalent == "\r" ? nil : .controlAccentColor
	}

	@objc private func press() {
		onPress()
	}

	private static func title(_ title: String, hint: String?, role: ButtonRole, size: ButtonSize) -> NSAttributedString {
		// The title takes the system's colour for the button.
		let words = NSMutableAttributedString(string: title, attributes: [
			.font: NSFont.systemFont(ofSize: NSFont.systemFontSize(for: size.controlSize)),
		])
		if let hint {
			// On a primary the hint takes the title's own colour, which AppKit
			// sets for the bezel it draws (white on the accent, dark on grey).
			var attributes: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 11)]
			if role != .primary {
				attributes[.foregroundColor] = dimmedText
			}
			words.append(NSAttributedString(string: "  \(hint)", attributes: attributes))
		}
		return words
	}

	/// The button text at 60 %, in light and dark alike.
	private static let dimmedText = NSColor(name: "GentboxDimmedControlText") { appearance in
		var color = NSColor.controlTextColor
		appearance.performAsCurrentDrawingAppearance {
			color = NSColor.controlTextColor.usingColorSpace(.sRGB) ?? .black
		}
		return color.withAlphaComponent(0.6)
	}
}

/// Labels as AppKit draws text: one line that truncates, or wrapping.
@MainActor
enum Label {
	static func line(_ text: String = "", size: CGFloat, weight: NSFont.Weight = .regular, color: NSColor = .labelColor) -> NSTextField {
		let field = NSTextField(labelWithString: text)
		field.font = .systemFont(ofSize: size, weight: weight)
		field.textColor = color
		field.lineBreakMode = .byTruncatingTail
		field.maximumNumberOfLines = 1
		field.cell?.truncatesLastVisibleLine = true
		return field
	}

	static func wrapping(_ text: String = "", font: NSFont, color: NSColor = .labelColor, maxLines: Int = 0) -> NSTextField {
		let field = NSTextField(wrappingLabelWithString: text)
		field.font = font
		field.textColor = color
		field.maximumNumberOfLines = maxLines
		field.cell?.truncatesLastVisibleLine = maxLines > 0
		field.isSelectable = false
		return field
	}

	/// AppKit insets a label's text by 2 points on either side; the frames
	/// below put the text itself where the layout says.
	static let inset: CGFloat = 2

	/// The frame for a label whose text starts at `x` and spans `width`.
	static func frame(x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) -> NSRect {
		NSRect(x: x - inset, y: y, width: width + 2 * inset, height: height)
	}

	/// A label's natural size, whatever its frame is now. `fittingSize`
	/// gives the same but solves it in a temporary Auto Layout engine on
	/// every call; `intrinsicContentSize` follows the current width.
	static func size(of field: NSTextField) -> NSSize {
		guard let cell = field.cell else { return field.fittingSize }
		let size = cell.cellSize(forBounds: NSRect(x: 0, y: 0, width: CGFloat.greatestFiniteMagnitude, height: .greatestFiniteMagnitude))
		return NSSize(width: ceil(size.width), height: ceil(size.height))
	}

	/// The width of a one-line label's text.
	static func textWidth(of field: NSTextField) -> CGFloat {
		ceil(size(of: field).width) - 2 * inset
	}

	/// The height `field` takes with its text `width` wide.
	static func height(of field: NSTextField, width: CGFloat) -> CGFloat {
		ceil(field.sizeThatFits(NSSize(width: width + 2 * inset, height: .greatestFiniteMagnitude)).height)
	}
}

/// An SF Symbol in a colour, at a point size.
@MainActor
enum Symbol {
	static func view(_ name: String, size: CGFloat, color: NSColor) -> NSImageView {
		let view = NSImageView()
		set(view, name, size: size, color: color)
		view.imageScaling = .scaleNone
		return view
	}

	/// Rows set their symbols on every change; a symbol image is looked up in
	/// the asset catalog once per name and size.
	static func set(_ view: NSImageView, _ name: String, size: CGFloat, color: NSColor) {
		let image = image(name, size: size)
		if view.image !== image {
			view.image = image
		}
		if view.contentTintColor != color {
			view.contentTintColor = color
		}
	}

	private static var images: [String: NSImage] = [:]

	private static func image(_ name: String, size: CGFloat) -> NSImage? {
		let key = "\(name) \(size)"
		if let image = images[key] { return image }
		let image = NSImage(systemSymbolName: name, accessibilityDescription: nil)?
			.withSymbolConfiguration(.init(pointSize: size, weight: .regular))
		images[key] = image
		return image
	}
}

/// A flipped view whose layer draws a background colour that follows the
/// appearance, and that lays out nothing itself.
@MainActor
class FlippedView: NSView {
	var fill: NSColor? {
		didSet {
			if fill != oldValue { needsDisplay = true }
		}
	}

	var cornerRadius: CGFloat = 0 {
		didSet { layer?.cornerRadius = cornerRadius }
	}

	override init(frame: NSRect) {
		super.init(frame: frame)
		wantsLayer = true
		layerContentsRedrawPolicy = .onSetNeedsDisplay
		// Laid out by hand: a size change resizes no subview, which would
		// lay them out again and render each control's SwiftUI graph.
		autoresizesSubviews = false
	}

	@available(*, unavailable)
	required init?(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	override var isFlipped: Bool { true }
	override var wantsUpdateLayer: Bool { true }

	override func updateLayer() {
		layer?.backgroundColor = fill?.cgColor
	}

	/// A part kept drawn at zero opacity (`setConcealed`) takes no clicks.
	override func hitTest(_ point: NSPoint) -> NSView? {
		guard let hit = super.hitTest(point) else { return nil }
		var view: NSView? = hit
		while let current = view, current !== self {
			if current.alphaValue == 0 { return self }
			view = current.superview
		}
		return hit
	}
}

extension NSView {
	/// Shown, or kept drawn at zero opacity, out of reach of clicks (in a
	/// `FlippedView`), focus and VoiceOver: showing it again costs no
	/// drawing, which `isHidden` would put off until then.
	func setConcealed(_ isConcealed: Bool) {
		alphaValue = isConcealed ? 0 : 1
		setInReach(!isConcealed)
	}

	/// In or out of reach of focus and VoiceOver. Out of reach, a control
	/// refuses focus: a panel that becomes key on macOS 15 focuses the first
	/// view that takes it, and a concealed reply field would take the keys
	/// meant for the card. A control's accessibility element is its cell (a
	/// button's role and actions are the cell's); the control view stays
	/// out, or it would be a second element, of no role, over the cell.
	/// Only what differs is set: on macOS 26 a control's state change
	/// renders its SwiftUI graph again in the next layout pass.
	func setInReach(_ isInReach: Bool) {
		guard let control = self as? NSControl, let cell = control.cell else {
			if isAccessibilityElement() != isInReach {
				setAccessibilityElement(isInReach)
			}
			return
		}
		if control.refusesFirstResponder == isInReach {
			control.refusesFirstResponder = !isInReach
		}
		if !isInReach, let window, window.firstResponder === control.currentEditor() ?? control {
			window.makeFirstResponder(nil)
		}
		if isAccessibilityElement() {
			setAccessibilityElement(false)
		}
		if cell.isAccessibilityElement() != isInReach {
			cell.setAccessibilityElement(isInReach)
		}
	}

	/// Moves the view only when it isn't there already: AppKit redoes its
	/// geometry bookkeeping on every frame set, even an unchanged one, which
	/// adds up over a long list on each change.
	func move(to origin: NSPoint) {
		if frame.origin != origin {
			setFrameOrigin(origin)
		}
	}
}
