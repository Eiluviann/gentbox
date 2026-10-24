import AppKit
import XCTest

/// A concealed control is no accessibility element at all, the view or its
/// cell; a shown one is exactly one, its cell, as AppKit makes it: a second
/// element of no role over the cell would take the hit tests meant for the
/// button.
@MainActor
final class ControlAccessibilityTests: XCTestCase {
	func testAConcealedControlIsNoElementAndAShownOneIsItsCell() throws {
		for control in [NSButton(title: "Dismiss", target: nil, action: nil), ReplyField()] as [NSControl] {
			let cell = try XCTUnwrap(control.cell)
			control.setAccessible(false)
			XCTAssertFalse(control.isAccessibilityElement())
			XCTAssertFalse(cell.isAccessibilityElement())
			control.setAccessible(true)
			XCTAssertFalse(control.isAccessibilityElement(), "the control view is a second element over its cell")
			XCTAssertTrue(cell.isAccessibilityElement())
			control.setAccessible(false)
			XCTAssertFalse(control.isAccessibilityElement())
			XCTAssertFalse(cell.isAccessibilityElement())
		}
	}
}
