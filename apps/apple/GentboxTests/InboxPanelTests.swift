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

	/// Spawn scales about the card's top centre: that point never moves, so
	/// the card grows down and out from under the status item.
	func testTheSpawnScaleHoldsTheCardsTopCentre() {
		let panel = InboxPanel(content: NSView())
		panel.place(card: NSRect(x: 900, y: 700, width: 380, height: 300), bottom: 0)
		let cardTopCentre = CGPoint(x: panel.frame.width / 2, y: panel.frame.height - InboxPanel.margin)
		for scale in [0.96, 0.98, 1.0] {
			let moved = cardTopCentre.applying(CATransform3DGetAffineTransform(panel.cardTopTransform(scale: scale)))
			XCTAssertEqual(moved.x, cardTopCentre.x, accuracy: 0.001, "scale \(scale)")
			XCTAssertEqual(moved.y, cardTopCentre.y, accuracy: 0.001, "scale \(scale)")
		}
	}

	/// The card's height moves in Core Animation from where it shows, on the
	/// content's curve; without motion it jumps, dropping any motion left.
	func testTheCardsHeightAnimatesFromWhereItShows() throws {
		let panel = InboxPanel(content: NSView())
		panel.place(card: NSRect(x: 900, y: 700, width: 380, height: 300), bottom: 0)
		// Shadow, surface, ring: the ring's own view stays on top.
		let ring = try XCTUnwrap(panel.contentView?.subviews.last?.layer?.sublayers?.first)
		XCTAssertEqual(ring.borderWidth, 0.5)
		XCTAssertEqual(panel.contentView?.subviews.count, 3)
		panel.setCardHeight(400, animated: true)
		XCTAssertEqual(panel.cardHeight, 400)
		let grow = try XCTUnwrap(ring.animation(forKey: "motion.bounds") as? CABasicAnimation)
		XCTAssertEqual((grow.fromValue as? NSValue)?.rectValue.height ?? 0, 300)
		XCTAssertEqual((grow.toValue as? NSValue)?.rectValue.height ?? 0, 400)
		XCTAssertEqual(grow.duration, Tokens.motionDuration, accuracy: 0.001)
		var control = [Float](repeating: 0, count: 2)
		grow.timingFunction?.getControlPoint(at: 1, values: &control)
		XCTAssertEqual(control, [0.42, 0], "ease in and out")

		panel.setCardHeight(250, animated: false)
		XCTAssertNil(ring.animation(forKey: "motion.bounds"))
		XCTAssertEqual(ring.bounds.height, 250)
	}

	func testSpawnAndDespawnFadeAndScaleAndReduceMotionOnlyFades() throws {
		let panel = InboxPanel(content: NSView())
		panel.place(card: NSRect(x: 900, y: 700, width: 380, height: 300), bottom: 0)
		panel.spawn(reduceMotion: false)
		let spawn = try XCTUnwrap(panel.contentView?.layer?.animation(forKey: "spawn") as? CAAnimationGroup)
		XCTAssertEqual(spawn.duration, 0.18, accuracy: 0.001)
		let grow = try XCTUnwrap(spawn.animations?.last as? CABasicAnimation)
		XCTAssertEqual((grow.fromValue as? NSValue)?.caTransform3DValue.m11 ?? 0, 0.96, accuracy: 0.001)

		panel.spawn(reduceMotion: true)
		let fade = try XCTUnwrap(panel.contentView?.layer?.animation(forKey: "spawn") as? CAAnimationGroup)
		let still = try XCTUnwrap(fade.animations?.last as? CABasicAnimation)
		XCTAssertEqual((still.fromValue as? NSValue)?.caTransform3DValue.m11 ?? 0, 1, accuracy: 0.001)
		XCTAssertEqual((still.toValue as? NSValue)?.caTransform3DValue.m11 ?? 0, 1, accuracy: 0.001)

		panel.despawn(reduceMotion: false) {}
		XCTAssertTrue(panel.isDespawning)
		XCTAssertTrue(panel.ignoresMouseEvents, "despawning ignores input")
		let out = try XCTUnwrap(panel.contentView?.layer?.animation(forKey: "spawn") as? CAAnimationGroup)
		XCTAssertEqual(out.duration, 0.12, accuracy: 0.001)
		let shrink = try XCTUnwrap(out.animations?.last as? CABasicAnimation)
		XCTAssertEqual((shrink.toValue as? NSValue)?.caTransform3DValue.m11 ?? 0, 0.98, accuracy: 0.001)

		panel.spawn(reduceMotion: false)
		XCTAssertFalse(panel.isDespawning, "a re-open turns the despawn back")
		XCTAssertFalse(panel.ignoresMouseEvents)
	}
}
