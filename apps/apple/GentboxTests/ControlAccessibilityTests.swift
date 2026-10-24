import AppKit
import XCTest

/// A concealed control is out of reach: no accessibility element, the view
/// or its cell, and no focus. A shown one is exactly one element, its cell,
/// as AppKit makes it (a second element of no role over the cell would take
/// the hit tests meant for the button), and takes focus again.
@MainActor
final class ControlAccessibilityTests: XCTestCase {
	func testAConcealedControlIsNoElementAndAShownOneIsItsCell() throws {
		for control in [NSButton(title: "Dismiss", target: nil, action: nil), ReplyField()] as [NSControl] {
			let cell = try XCTUnwrap(control.cell)
			control.setInReach(false)
			XCTAssertFalse(control.isAccessibilityElement())
			XCTAssertFalse(cell.isAccessibilityElement())
			control.setInReach(true)
			XCTAssertFalse(control.isAccessibilityElement(), "the control view is a second element over its cell")
			XCTAssertTrue(cell.isAccessibilityElement())
			control.setInReach(false)
			XCTAssertFalse(control.isAccessibilityElement())
			XCTAssertFalse(cell.isAccessibilityElement())
		}
	}

	/// On macOS 15 a panel that becomes key focuses the first view in its
	/// key loop that takes focus: a concealed reply field must not be it, or
	/// it takes the card's R as text. One that has focus gives it up.
	func testAConcealedReplyFieldTakesNoFocus() {
		let field = ReplyField()
		let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 200), styleMask: .borderless, backing: .buffered, defer: true)
		window.contentView?.addSubview(field)
		XCTAssertTrue(window.makeFirstResponder(field))
		field.setInReach(false)
		XCTAssertFalse(field.acceptsFirstResponder)
		XCTAssertNil(field.currentEditor(), "it kept focus")
		field.setInReach(true)
		XCTAssertTrue(field.acceptsFirstResponder)
	}
}
