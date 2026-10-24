import AppKit

/// The reply box (the user's decision): the card's full width, one line when
/// it starts, a line more as the text is about to wrap, three at most, and
/// past that it scrolls inside. ↩ sends; ⌥↩ breaks a line.
@MainActor
final class ReplyField: NSTextField {
	static let maxLines = 3
	/// A line takes the next one this close to the edge, a caret's width
	/// before the field editor itself would wrap.
	static let wrapAllowance: CGFloat = 2

	init() {
		super.init(frame: .zero)
		Self.configure(self)
	}

	/// What makes it a bezelled field that wraps its text.
	private static func configure(_ field: NSTextField) {
		field.bezelStyle = .roundedBezel
		field.controlSize = .regular
		field.usesSingleLineMode = false
		field.lineBreakMode = .byWordWrapping
		field.maximumNumberOfLines = 0
		field.cell?.wraps = true
		field.cell?.isScrollable = false
	}

	@available(*, unavailable)
	required init?(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	/// How many lines its text takes `width` wide, from 1 up to `maxLines`.
	func lineCount(width: CGFloat) -> Int {
		let text = currentEditor()?.string ?? stringValue
		if let counted, counted.text == text, counted.width == width {
			return counted.lines
		}
		let metrics = lineMetrics(width: width)
		let height = measuredHeight(text.isEmpty ? "Hg" : text, width: width - Self.wrapAllowance)
		let lines = min(max(Int(((height - metrics.firstLine) / metrics.nextLine).rounded()) + 1, 1), Self.maxLines)
		counted = (text, width, lines)
		return lines
	}

	/// Its height `width` wide: the bezel around `lineCount(width:)` lines.
	func height(width: CGFloat) -> CGFloat {
		let metrics = lineMetrics(width: width)
		return ceil(metrics.firstLine + CGFloat(lineCount(width: width) - 1) * metrics.nextLine)
	}

	// MARK: Measuring

	/// A field like it that is never shown, to measure text in without
	/// touching the text being edited. A copy of the cell, or a cell made on
	/// its own, measures one line whatever the text.
	private static let measurer: NSTextField = {
		let field = NSTextField()
		configure(field)
		return field
	}()

	/// The last count, for the layout pass that asks twice.
	private var counted: (text: String, width: CGFloat, lines: Int)?
	private var metrics: (width: CGFloat, font: NSFont?, firstLine: CGFloat, nextLine: CGFloat)?

	/// The bezel around one line, and what each further line adds.
	private func lineMetrics(width: CGFloat) -> (firstLine: CGFloat, nextLine: CGFloat) {
		if let metrics, metrics.width == width, metrics.font == font {
			return (metrics.firstLine, metrics.nextLine)
		}
		let one = measuredHeight("Hg", width: width)
		let two = measuredHeight("Hg\nHg", width: width)
		metrics = (width, font, one, max(two - one, 1))
		return (one, max(two - one, 1))
	}

	private func measuredHeight(_ text: String, width: CGFloat) -> CGFloat {
		let measurer = Self.measurer
		measurer.font = font
		measurer.stringValue = text
		return measurer.cell?.cellSize(forBounds: NSRect(x: 0, y: 0, width: width, height: .greatestFiniteMagnitude)).height ?? fittingSize.height
	}
}
