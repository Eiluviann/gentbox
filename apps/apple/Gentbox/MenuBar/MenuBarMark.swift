import AppKit

/// The Gentbox mark for the menu bar: `bubble.left` with a `>_` prompt
/// drawn as strokes (design: `mac-menubar-icons`). A template image, so the
/// system tints it for light and dark menu bars; the one exception is the
/// red dot while an agent waits on you.
enum MenuBarMark {
	enum State: Equatable {
		/// Nothing open.
		case idle
		/// Something you have not seen: a dot in the mark's own colour.
		case unread
		/// Agents wait on you: a red dot, and the count beside the mark.
		case blocked
	}

	/// The mark's frame is 100 units square, as in `design/tools`.
	private static let promptStrokes: [[CGPoint]] = [
		[CGPoint(x: 28, y: 24), CGPoint(x: 42, y: 37.5), CGPoint(x: 28, y: 51)],
		[CGPoint(x: 50, y: 51), CGPoint(x: 68, y: 51)],
	]
	private static let promptWidth: CGFloat = 7
	private static let dotCenter = CGPoint(x: 84, y: 16)
	private static let dotRadius: CGFloat = 14
	/// The gap cut around the dot.
	private static let knockWidth: CGFloat = 9
	private static let pointSize: CGFloat = 18

	static func image(for state: State) -> NSImage {
		let size = NSSize(width: pointSize, height: pointSize)
		let image = NSImage(size: size, flipped: true) { rect in
			let ink: NSColor = state == .blocked ? .labelColor : .black
			draw(in: rect, ink: ink, state: state)
			return true
		}
		image.isTemplate = state != .blocked
		image.accessibilityDescription = "Gentbox"
		return image
	}

	private static func draw(in rect: NSRect, ink: NSColor, state: State) {
		guard let context = NSGraphicsContext.current?.cgContext else { return }
		let scale = rect.width / 100
		drawBubble(in: rect, ink: ink)
		context.saveGState()
		context.scaleBy(x: scale, y: scale)
		ink.setStroke()
		for stroke in promptStrokes {
			let path = NSBezierPath()
			path.lineWidth = promptWidth
			path.lineCapStyle = .round
			path.lineJoinStyle = .round
			for (index, point) in stroke.enumerated() {
				if index == 0 { path.move(to: point) } else { path.line(to: point) }
			}
			path.stroke()
		}
		if state != .idle {
			let knock = dotRadius + knockWidth / 2
			context.setBlendMode(.clear)
			NSBezierPath(ovalIn: square(around: dotCenter, radius: knock)).fill()
			context.setBlendMode(.normal)
			(state == .blocked ? NSColor.systemRed : ink).setFill()
			NSBezierPath(ovalIn: square(around: dotCenter, radius: dotRadius)).fill()
		}
		context.restoreGState()
	}

	/// The system's `bubble.left`, tinted with `ink`.
	private static func drawBubble(in rect: NSRect, ink: NSColor) {
		let configuration = NSImage.SymbolConfiguration(pointSize: pointSize, weight: .regular)
			.applying(NSImage.SymbolConfiguration(paletteColors: [ink]))
		guard let symbol = NSImage(systemSymbolName: "bubble.left", accessibilityDescription: nil)?
			.withSymbolConfiguration(configuration)
		else { return }
		let fitted = aspectFit(symbol.size, in: rect)
		symbol.draw(in: fitted)
	}

	private static func aspectFit(_ size: NSSize, in rect: NSRect) -> NSRect {
		let scale = min(rect.width / size.width, rect.height / size.height)
		let fitted = NSSize(width: size.width * scale, height: size.height * scale)
		return NSRect(
			x: rect.midX - fitted.width / 2,
			y: rect.midY - fitted.height / 2,
			width: fitted.width,
			height: fitted.height,
		)
	}

	private static func square(around center: CGPoint, radius: CGFloat) -> NSRect {
		NSRect(x: center.x - radius, y: center.y - radius, width: radius * 2, height: radius * 2)
	}
}
