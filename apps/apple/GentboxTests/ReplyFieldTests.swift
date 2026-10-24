import AppKit
import XCTest

/// The reply box spans the card's width at one line, takes a line more as
/// the text is about to wrap, stops at three and scrolls inside after that.
@MainActor
final class ReplyFieldTests: XCTestCase {
	/// The card body's width (`RowMetrics.bodyWidth`).
	private let width: CGFloat = 326

	func testItStartsAtOneLine() {
		let field = ReplyField()
		XCTAssertEqual(field.lineCount(width: width), 1)
		let empty = field.height(width: width)
		field.stringValue = "Run only the auth suite"
		XCTAssertEqual(field.lineCount(width: width), 1)
		XCTAssertEqual(field.height(width: width), empty)
		// The regular rounded bezel's own height around one line.
		XCTAssertEqual(empty, 22, accuracy: 2)
	}

	func testItGrowsALineAtATimeUpToThree() {
		let field = ReplyField()
		let one = field.height(width: width)
		field.stringValue = "First\nSecond"
		let two = field.height(width: width)
		field.stringValue = "First\nSecond\nThird"
		let three = field.height(width: width)
		XCTAssertEqual(field.lineCount(width: width), 3)
		XCTAssertGreaterThan(two - one, 10)
		XCTAssertEqual(three - two, two - one, accuracy: 1)
	}

	func testTypingWrapsIntoTheNextLineAndStopsAtThree() {
		let field = ReplyField()
		var counts: [Int] = []
		var text = ""
		for word in Array(repeating: "Run only the auth suite, not the whole package", count: 8).joined(separator: " ").split(separator: " ") {
			text += text.isEmpty ? String(word) : " \(word)"
			field.stringValue = text
			counts.append(field.lineCount(width: width))
		}
		XCTAssertEqual(counts.first, 1)
		XCTAssertEqual(counts.last, 3)
		XCTAssertEqual(counts, counts.sorted(), "a line count never drops while typing on")
		XCTAssertEqual(Set(counts), [1, 2, 3], "it goes 1 → 2 → 3 one line at a time")
		// A line is taken before the text reaches the edge, never after it.
		let firstTwo = counts.firstIndex(of: 2) ?? 0
		let words = text.split(separator: " ")
		let lastOneLine = words.prefix(firstTwo).joined(separator: " ")
		let unwrapped = (lastOneLine as NSString).size(withAttributes: [.font: field.font as Any]).width
		XCTAssertLessThan(unwrapped, width)
	}

	func testPastThreeLinesItStaysThreeLinesTallAndScrollsInside() throws {
		let field = ReplyField()
		field.stringValue = "First\nSecond\nThird"
		let three = field.height(width: width)
		field.stringValue = (1 ... 6).map { "Line \($0)" }.joined(separator: "\n")
		XCTAssertEqual(field.lineCount(width: width), 3)
		XCTAssertEqual(field.height(width: width), three)

		// Edited in a window that is never shown: the caret at the end, the
		// field editor taller than the field and scrolled down to it.
		let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 200), styleMask: .borderless, backing: .buffered, defer: true)
		field.frame = NSRect(x: 10, y: 10, width: width, height: three)
		window.contentView?.addSubview(field)
		XCTAssertTrue(window.makeFirstResponder(field))
		let editor = try XCTUnwrap(field.currentEditor() as? NSTextView)
		editor.setSelectedRange(NSRange(location: (editor.string as NSString).length, length: 0))
		editor.scrollRangeToVisible(editor.selectedRange())
		XCTAssertGreaterThan(editor.frame.height, field.bounds.height)
		XCTAssertGreaterThan(editor.visibleRect.minY, 0)
		XCTAssertEqual(field.height(width: width), three, "editing never grows it past three lines")
		window.makeFirstResponder(nil)
	}
}
