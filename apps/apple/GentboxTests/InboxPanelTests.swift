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

	/// With the menu bar hidden automatically the visible frame reaches the
	/// top of the screen; opened from the revealed menu bar, the card still
	/// hangs under the status item, never over the menu bar's items.
	func testWithAnAutoHiddenMenuBarItHangsUnderTheItem() {
		let screen = NSRect(x: 0, y: 0, width: 1728, height: 1117)
		let item = NSRect(x: 1300, y: 1117 - 33, width: 36, height: 33)
		let frame = InboxPanel.frame(size: NSSize(width: 380, height: 300), under: item, in: screen, screen: screen)
		XCTAssertEqual(frame.maxY, item.minY - InboxPanel.gap)
		XCTAssertEqual(frame.midX, item.midX)
		XCTAssertEqual(InboxPanel.maxHeight(under: item, in: screen, screen: screen), item.minY - InboxPanel.gap - InboxPanel.gap)
		let panel = InboxPanel(content: NSView())
		panel.place(card: frame, maxCardHeight: 700)
		XCTAssertLessThanOrEqual(panel.frame.maxY, item.minY, "the window covers the menu bar")
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
		panel.place(card: NSRect(x: 900, y: 700, width: 380, height: 300), maxCardHeight: 700)
		let cardTopCentre = CGPoint(x: panel.frame.width / 2, y: panel.frame.height - InboxPanel.topRoom)
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
		panel.place(card: NSRect(x: 900, y: 700, width: 380, height: 300), maxCardHeight: 700)
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
		panel.place(card: NSRect(x: 900, y: 700, width: 380, height: 300), maxCardHeight: 700)
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
		XCTAssertFalse(panel.ignoresMouseEvents, "input again, wherever the pointer is")
	}

	/// The window hugs the card, with room for its shadow, so the window
	/// server gives the card every click with no pointer tracking, and
	/// everything below the shadow passes through. Its top never moves; it
	/// grows before a motion that grows the card and shrinks after one that
	/// shrinks it, so the card always fits.
	func testTheWindowHugsTheCardAndHoldsItsTop() {
		let panel = InboxPanel(content: NSView())
		let card = NSRect(x: 900, y: 400, width: 380, height: 300)
		panel.place(card: card, maxCardHeight: 700)
		XCTAssertEqual(panel.frame.maxY, card.maxY + InboxPanel.topRoom)
		XCTAssertEqual(panel.frame.height, 300 + InboxPanel.topRoom + InboxPanel.bottomRoom)
		XCTAssertEqual(panel.cardFrameOnScreen, card)

		panel.setCardHeight(400, animated: true)
		XCTAssertEqual(panel.frame.maxY, card.maxY + InboxPanel.topRoom)
		XCTAssertEqual(panel.frame.height, 400 + InboxPanel.topRoom + InboxPanel.bottomRoom, "grown before the motion")

		panel.setCardHeight(250, animated: true)
		XCTAssertEqual(panel.frame.height, 400 + InboxPanel.topRoom + InboxPanel.bottomRoom, "kept while the card shrinks")
		let settled = expectation(description: "shrunk once the motion is over")
		DispatchQueue.main.asyncAfter(deadline: .now() + Tokens.motionDuration + 0.1) { settled.fulfill() }
		wait(for: [settled], timeout: 2)
		XCTAssertEqual(panel.frame.height, 250 + InboxPanel.topRoom + InboxPanel.bottomRoom)
		XCTAssertEqual(panel.frame.maxY, card.maxY + InboxPanel.topRoom)

		panel.setCardHeight(180, animated: false)
		XCTAssertEqual(panel.frame.height, 180 + InboxPanel.topRoom + InboxPanel.bottomRoom)
	}

	/// The window reaches no higher than the gap above the card: above that
	/// is the menu bar, whose items must take their own clicks while the
	/// inbox shows (a click there switches to them, as between menus).
	func testTheWindowLeavesTheMenuBarAlone() {
		let screen = NSRect(x: 0, y: 0, width: 1512, height: 944)
		let visible = NSRect(x: 0, y: 0, width: 1512, height: 944 - 33)
		let item = NSRect(x: 1300, y: 944 - 33, width: 36, height: 33)
		let card = InboxPanel.frame(size: NSSize(width: 380, height: 300), under: item, in: visible)
		let panel = InboxPanel(content: NSView())
		panel.place(card: card, maxCardHeight: 700)
		XCTAssertLessThanOrEqual(panel.frame.maxY, card.maxY + InboxPanel.gap)
		XCTAssertLessThanOrEqual(panel.frame.maxY, visible.maxY, "the window covers the menu bar")
		XCTAssertLessThan(panel.frame.maxY, screen.maxY)
	}

	/// The window's top cuts the shadow's faint top, so the shadow image
	/// fades to nothing there: no line of shadow under the menu bar.
	func testTheShadowFadesOutAtTheWindowsTop() throws {
		for scale in [1.0, 2.0] as [CGFloat] {
			let image = NSBitmapImageRep(cgImage: InboxPanel.shadowImage(scale: scale))
			let edgeRow = Int((InboxPanel.shadowRowsAboveWindowTop * scale).rounded(.up))
			for x in [image.pixelsWide / 2, image.pixelsWide / 3] {
				for row in 0 ... edgeRow {
					let alpha = try XCTUnwrap(image.colorAt(x: x, y: row)).alphaComponent
					// The window's first row straddles the fade's start: 1/255 at most.
					XCTAssertLessThanOrEqual(alpha, row < edgeRow ? 0 : 1.0 / 255, "row \(row) at \(x), scale \(scale)")
				}
				let underTheCard = Int(((InboxPanel.shadowRowsAboveWindowTop + InboxPanel.topRoom + 8) * scale).rounded())
				XCTAssertGreaterThan(try XCTUnwrap(image.colorAt(x: x, y: underTheCard)).alphaComponent, 0.05, "the shadow faded where the card covers it")
			}
		}
	}

	/// A shrink that a later growth overtakes never cuts the card short.
	func testAGrowthOvertakesAPendingShrink() {
		let panel = InboxPanel(content: NSView())
		panel.place(card: NSRect(x: 900, y: 400, width: 380, height: 300), maxCardHeight: 700)
		panel.setCardHeight(200, animated: true)
		panel.setCardHeight(350, animated: true)
		let settled = expectation(description: "past the first motion")
		DispatchQueue.main.asyncAfter(deadline: .now() + Tokens.motionDuration + 0.1) { settled.fulfill() }
		wait(for: [settled], timeout: 2)
		XCTAssertEqual(panel.frame.height, 350 + InboxPanel.topRoom + InboxPanel.bottomRoom)
	}

	/// Becoming key leaves no text focused but an open reply box: a
	/// selectable code line or a concealed reply field would take R and ↩.
	func testBecomingKeyFocusesOnlyAnOpenReply() {
		let content = NSView(frame: NSRect(x: 0, y: 0, width: 380, height: 300))
		let code = NSTextField(labelWithString: "cargo test -p gentbox-hub")
		code.isSelectable = true
		let concealed = ReplyField()
		concealed.alphaValue = 0
		let reply = ReplyField()
		for view in [code, concealed, reply] {
			content.addSubview(view)
		}
		let panel = InboxPanel(content: content)
		panel.place(card: NSRect(x: 900, y: 400, width: 380, height: 300), maxCardHeight: 700)
		for view in [code, concealed] {
			XCTAssertTrue(panel.makeFirstResponder(view))
			panel.becomeKey()
			XCTAssertTrue(panel.firstResponder === panel, "\(view) kept focus")
		}
		XCTAssertTrue(panel.makeFirstResponder(reply))
		panel.becomeKey()
		XCTAssertNotNil(reply.currentEditor(), "the open reply lost focus")
	}

	/// The room around the card is the panel's own, not the card's: a click
	/// there closes the panel, as a click anywhere else would.
	func testAClickBesideTheCardCloses() throws {
		let panel = InboxPanel(content: NSView())
		panel.place(card: NSRect(x: 900, y: 400, width: 380, height: 300), maxCardHeight: 700)
		var closes = 0
		panel.onClose = { closes += 1 }
		let stage = try XCTUnwrap(panel.contentView)
		let belowTheCard = NSPoint(x: InboxPanel.margin + 100, y: InboxPanel.topRoom + 300 + 10)
		let hit = try XCTUnwrap(stage.hitTest(stage.convert(belowTheCard, to: stage.superview)))
		XCTAssertTrue(hit === stage)
		let click = try XCTUnwrap(NSEvent.mouseEvent(
			with: .leftMouseDown, location: stage.convert(belowTheCard, to: nil), modifierFlags: [], timestamp: 0,
			windowNumber: panel.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1,
		))
		hit.mouseDown(with: click)
		XCTAssertEqual(closes, 1)
	}
}
