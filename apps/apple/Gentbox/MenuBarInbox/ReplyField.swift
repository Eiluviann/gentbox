import AppKit

/// The reply box (the user's decision): the card's full width, one line when
/// it starts, a line more as the text is about to wrap, three at most, and
/// past that it scrolls inside. ↩ sends; ⌥↩ breaks a line.
@MainActor
final class ReplyField: NSTextField {
	static let maxLines = 3
	/// A line is taken this much before the text would reach the edge: the
	/// field editor's own line padding (up to 5 points a side) and a caret.
	static let wrapAllowance: CGFloat = 12

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
		let lines = min(max(Self.laidOutLines(text, font: font, width: textWidth(width) - Self.wrapAllowance), 1), Self.maxLines)
		counted = (text, width, lines)
		return lines
	}

	/// Its height `width` wide: the bezel around `lineCount(width:)` lines.
	func height(width: CGFloat) -> CGFloat {
		let metrics = measure(width: width)
		return ceil(metrics.oneLine + CGFloat(lineCount(width: width) - 1) * metrics.line)
	}

	// MARK: Measuring

	/// The last count, for the layout pass that asks twice.
	private var counted: (text: String, width: CGFloat, lines: Int)?

	/// The one-line height and line height last measured, for its font and
	/// width: measuring sets the measurer's font, which on macOS 26 updates
	/// that field's own SwiftUI graph, so it is done once per change.
	private var measured: (font: NSFont?, width: CGFloat, oneLine: CGFloat, line: CGFloat)?

	/// A field like it that is never shown, measured with one line of text:
	/// its own cell holds the text being edited. (A bezelled cell measures
	/// one line whatever the text on macOS 15, so lines are counted apart.)
	private static let measurer: NSTextField = {
		let field = NSTextField()
		configure(field)
		field.stringValue = "Hg"
		return field
	}()

	/// The bezel around one line, and what each line past the first adds.
	private func measure(width: CGFloat) -> (oneLine: CGFloat, line: CGFloat) {
		if let measured, measured.font == font, measured.width == width {
			return (measured.oneLine, measured.line)
		}
		if Self.measurer.font != font {
			Self.measurer.font = font
		}
		let oneLine = Self.measurer.cell?.cellSize(forBounds: NSRect(x: 0, y: 0, width: width, height: .greatestFiniteMagnitude)).height
			?? fittingSize.height
		let line = NSLayoutManager().defaultLineHeight(for: font ?? .systemFont(ofSize: NSFont.systemFontSize))
		measured = (font, width, oneLine, line)
		return (oneLine, line)
	}

	/// The width the text has inside the bezel.
	private func textWidth(_ width: CGFloat) -> CGFloat {
		Self.measurer.cell?.drawingRect(forBounds: NSRect(x: 0, y: 0, width: width, height: 22)).width ?? width
	}

	/// The lines `text` takes `width` wide, as TextKit lays it out: a line
	/// break at the end starts one more.
	private static func laidOutLines(_ text: String, font: NSFont?, width: CGFloat) -> Int {
		guard !text.isEmpty else { return 1 }
		let storage = NSTextStorage(string: text, attributes: [.font: font ?? .systemFont(ofSize: NSFont.systemFontSize)])
		let container = NSTextContainer(size: NSSize(width: max(width, 1), height: .greatestFiniteMagnitude))
		container.lineFragmentPadding = 0
		let manager = NSLayoutManager()
		manager.addTextContainer(container)
		storage.addLayoutManager(manager)
		var lines = 0
		manager.enumerateLineFragments(forGlyphRange: manager.glyphRange(for: container)) { _, _, _, _, _ in lines += 1 }
		return lines + (manager.extraLineFragmentTextContainer == nil ? 0 : 1)
	}
}
