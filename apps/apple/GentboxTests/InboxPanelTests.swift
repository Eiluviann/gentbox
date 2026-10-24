import AppKit
import XCTest

/// The inbox panel must never look like a standard window: tiling window
/// managers (AeroSpace) tile anything with the `AXStandardWindow` subrole, a
/// title and the usual buttons.
@MainActor
final class InboxPanelTests: XCTestCase {
	func testThePanelIsNoStandardWindow() {
		let panel = InboxPanel(content: NSView())
		XCTAssertNotEqual(panel.accessibilitySubrole(), .standardWindow)
		XCTAssertFalse(panel.styleMask.contains(.titled))
		XCTAssertFalse(panel.styleMask.contains(.resizable))
		XCTAssertFalse(panel.styleMask.contains(.closable))
		XCTAssertTrue(panel.styleMask.contains(.nonactivatingPanel))
		XCTAssertNil(panel.standardWindowButton(.closeButton))
		XCTAssertEqual(panel.level, .popUpMenu)
		XCTAssertTrue(
			panel.collectionBehavior.isSuperset(of: [.canJoinAllSpaces, .transient, .ignoresCycle, .fullScreenAuxiliary]),
		)
		XCTAssertTrue(panel.canBecomeKey)
		XCTAssertFalse(panel.canBecomeMain)
	}

	func testItHangsUnderTheItemInsideTheScreen() {
		let screen = NSRect(x: 0, y: 0, width: 1512, height: 944)
		let item = NSRect(x: 1300, y: 944, width: 36, height: 24)
		let frame = InboxPanel.frame(size: NSSize(width: 380, height: 500), under: item, in: screen)
		XCTAssertEqual(frame.maxY, 940)
		XCTAssertEqual(frame.midX, 1318)
		XCTAssertEqual(frame.height, 500)

		// Near the right edge it moves in; too tall, it shrinks to fit.
		let corner = NSRect(x: 1490, y: 944, width: 20, height: 24)
		let clamped = InboxPanel.frame(size: NSSize(width: 380, height: 2000), under: corner, in: screen)
		XCTAssertEqual(clamped.maxX, 1508)
		XCTAssertEqual(clamped.minY, 4)
	}

	func testWithTheMenuBarHiddenItHangsFromTheTop() {
		let screen = NSRect(x: 0, y: 0, width: 1512, height: 982)
		let hiddenItem = NSRect(x: 1300, y: 1010, width: 36, height: 24)
		let frame = InboxPanel.frame(size: NSSize(width: 380, height: 300), under: hiddenItem, in: screen)
		XCTAssertEqual(frame.maxY, 978)
	}

	/// While the menu bar is hidden or not laid out, the status item's window
	/// reports frames like (0, 0) or a y below the screen: the panel then
	/// hangs at the top right instead of collapsing to nothing.
	func testABogusAnchorStillGivesAFullPanelAtTheTopRight() {
		let screen = NSRect(x: 0, y: 0, width: 1728, height: 1085)
		for anchor in [NSRect(x: 0, y: 0, width: 34, height: 0), NSRect(x: 0, y: -28, width: 34, height: 33)] {
			let frame = InboxPanel.frame(size: NSSize(width: 380, height: 175), under: anchor, in: screen)
			XCTAssertEqual(frame.size, NSSize(width: 380, height: 175), "\(anchor)")
			XCTAssertEqual(frame.maxY, 1081, "\(anchor)")
			XCTAssertEqual(frame.maxX, 1724, "\(anchor)")
		}
	}

	/// Content taller than this scrolls: the panel reaches down to the
	/// bottom of the visible frame and no further.
	func testItStopsAtTheBottomOfTheScreen() {
		let screen = NSRect(x: 0, y: 40, width: 1512, height: 904)
		let item = NSRect(x: 1300, y: 944, width: 36, height: 24)
		XCTAssertEqual(InboxPanel.maxHeight(under: item, in: screen), 940 - 40 - 4)
		let tall = InboxPanel.frame(size: NSSize(width: 380, height: 5000), under: item, in: screen)
		XCTAssertEqual(tall.height, InboxPanel.maxHeight(under: item, in: screen))
	}
}
