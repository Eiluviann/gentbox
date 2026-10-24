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
		ceil(oneLineHeight(width: width) + CGFloat(lineCount(width: width) - 1) * lineHeight)
	}

	// MARK: Measuring

	/// The last count, for the layout pass that asks twice.
	private var counted: (text: String, width: CGFloat, lines: Int)?

	/// What each line past the first adds.
	private var lineHeight: CGFloat {
		NSLayoutManager().defaultLineHeight(for: font ?? .systemFont(ofSize: NSFont.systemFontSize))
	}

	/// A field like it that is never shown, measured with one line of text:
	/// its own cell holds the text being edited. (A bezelled cell measures
	/// one line whatever the text on macOS 15, so lines are counted apart.)
	private static let measurer: NSTextField = {
		let field = NSTextField()
		configure(field)
		field.stringValue = "Hg"
		return field
	}()

	/// The bezel around one line.
	private func oneLineHeight(width: CGFloat) -> CGFloat {
		Self.measurer.font = font
		return Self.measurer.cell?.cellSize(forBounds: NSRect(x: 0, y: 0, width: width, height: .greatestFiniteMagnitude)).height
			?? fittingSize.height
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
