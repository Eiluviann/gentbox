import AppKit
import GentboxCore
import QuartzCore
import XCTest

/// The inbox's content view, driven by a playground inbox in a window that
/// says it is shown but is never ordered in: each change runs as in the open
/// panel, and its frames are read from the layers' motions (ADR 0013).
@MainActor
final class InboxContentMotionTests: XCTestCase {
	/// Visible to the views, never on screen.
	private final class ShownWindow: NSWindow {
		override var isVisible: Bool {
			true
		}
	}

	@MainActor
	private struct Harness {
		let clock: ManualClock
		let playground: PlaygroundInbox
		let inbox: InboxModel
		let content: InboxContentView
		let window: NSWindow

		var scroller: ListScrollView? {
			Self.descendant(of: content)
		}

		/// The list section's slot: the scroll view sits in its content.
		var listSlot: SlotView? {
			scroller?.superview?.superview as? SlotView
		}

		/// The list's rows, top to bottom.
		var rows: [InboxRowView] {
			(scroller?.documentView?.subviews ?? []).compactMap { $0 as? InboxRowView }.filter { !$0.isHidden }
				.sorted { $0.frame.minY < $1.frame.minY }
		}

		/// The sections shown, top to bottom.
		var sectionSlots: [SlotView] {
			content.subviews.compactMap { $0 as? SlotView }.filter { !$0.isHidden }.sorted { $0.frame.minY < $1.frame.minY }
		}

		private static func descendant<View: NSView>(of view: NSView) -> View? {
			for subview in view.subviews {
				if let match = subview as? View { return match }
				if let match: View = descendant(of: subview) { return match }
			}
			return nil
		}
	}

	/// A text field's text and opacity fade, read as its change starts.
	private struct Ink {
		let text: String
		let opacity: CGFloat
		let fade: CABasicAnimation?
	}

	private static let tolerance: CGFloat = 1e-6
	private static let timeout: TimeInterval = 5
	/// Far from 0, which Core Animation reads as "now".
	private static let origin: TimeInterval = 1000

	private var cardChanges: [CardChange] = []
	/// Called as the card is told of a change, when the change's animations
	/// are on the layers: off screen, Core Animation drops them at the next
	/// commit.
	private var onCardChange: ((CardChange) -> Void)?

	// MARK: Handing the list over

	func testThePopulatedListFadesInAsOneSection() async throws {
		let harness = try await makeHarness()
		try await populate(harness)
		let listSlot = try XCTUnwrap(harness.listSlot)
		let listMotion = try XCTUnwrap(listSlot.contentMotion, "the list section comes in")
		let midpoint = listMotion.start + listMotion.spec.delay + listMotion.spec.duration / 2
		let shownList = listSlot.shownContent(at: midpoint)
		XCTAssertGreaterThan(shownList.opacity, 0)
		XCTAssertLessThan(shownList.opacity, 1, "the section fades in")
		XCTAssertFalse(harness.rows.isEmpty)
		for row in harness.rows {
			let shownRow = row.shownContent(at: midpoint)
			XCTAssertEqual(shownRow.opacity, 1, "row \(row.messageId) shows whole in the section's one fade")
			XCTAssertEqual(shownRow.scale, 1, accuracy: Self.tolerance, "row \(row.messageId) zooms with the section only")
		}
	}

	func testTheClearedListFadesOutAsOneSection() async throws {
		let harness = try await makeHarness()
		try await populate(harness)
		harness.content.settle()
		let listedRows = harness.rows
		XCTAssertFalse(listedRows.isEmpty)
		await harness.playground.clear()
		harness.inbox.requestRead()
		try await waitUntil("the list section leaves") { harness.listSlot?.isLeaving == true }
		let listSlot = try XCTUnwrap(harness.listSlot)
		let listMotion = try XCTUnwrap(listSlot.contentMotion)
		let midpoint = listMotion.start + listMotion.spec.delay + listMotion.spec.duration / 2
		let shownList = listSlot.shownContent(at: midpoint)
		XCTAssertGreaterThan(shownList.opacity, 0)
		XCTAssertLessThan(shownList.opacity, 1, "the section fades out")
		for row in listedRows {
			let shownRow = row.shownContent(at: midpoint)
			XCTAssertEqual(shownRow.opacity, 1, "row \(row.messageId) shows whole in the section's one fade")
			XCTAssertEqual(shownRow.scale, 1, accuracy: Self.tolerance, "row \(row.messageId) zooms with the section only")
		}
	}

	func testTheListZoomsAboutItsCentreTheFirstTimeItShows() async throws {
		let harness = try await makeHarness()
		var zoom: CABasicAnimation?
		var listBounds = NSRect.zero
		var listAnchor = CGPoint.zero
		onCardChange = { change in
			guard case .moving = change, let layer = harness.listSlot?.content.layer else { return }
			zoom = layer.animation(forKey: ContentMotion.transformKey) as? CABasicAnimation
			listBounds = layer.bounds
			listAnchor = layer.anchorPoint
		}
		try await populate(harness)
		let from = try XCTUnwrap((try XCTUnwrap(zoom, "the list zooms in").fromValue as? NSValue)?.caTransform3DValue)
		XCTAssertLessThan(from.m11, 1, "the list comes in zoomed out")
		XCTAssertGreaterThan(listBounds.height, 0, "the list is laid out")
		// A scale by s about (x, y) from the anchor moves the anchor by
		// (x, y) * (1 - s).
		let centre = CGPoint(x: from.m41 / (1 - from.m11), y: from.m42 / (1 - from.m22))
		XCTAssertEqual(centre.x, listBounds.width * (0.5 - listAnchor.x), accuracy: 1e-3)
		XCTAssertEqual(centre.y, listBounds.height * (0.5 - listAnchor.y), accuracy: 1e-3)
	}

	// MARK: Rows

	func testEveryRowComesWithItsMoreButton() async throws {
		let harness = try await makeHarness()
		var rowsWithoutButton: [String]?
		onCardChange = { change in
			guard case .moving = change, rowsWithoutButton == nil else { return }
			// As the rows are made, before any quiet turn builds ahead.
			rowsWithoutButton = harness.rows.filter { !Self.hasMoreButton($0) }.map(\.messageId)
		}
		try await populate(harness)
		XCTAssertEqual(rowsWithoutButton, [], "a hover only shows it")
	}

	func testASupersededMessagesHeaderCrossfadesOnTheMotionClock() async throws {
		let harness = try await makeHarness()
		try await populate(harness)
		harness.content.settle()
		let row = try XCTUnwrap(harness.rows.first { !$0.rowContent.isExpanded })
		let oldId = row.messageId
		let oldTitle = row.rowContent.message.title
		var start: TimeInterval?
		var inks: [Ink] = []
		var transitionKeys: [String] = []
		onCardChange = { change in
			guard case .moving = change, start == nil, row.messageId != oldId else { return }
			start = harness.clock.now
			transitionKeys = Self.layers(in: row).flatMap { layer in
				(layer.animationKeys() ?? []).filter { layer.animation(forKey: $0) is CATransition }
			}
			inks = Self.descendants(of: row).compactMap { (field: NSTextField) in
				Ink(text: field.stringValue, opacity: field.alphaValue, fade: field.layer?.animation(forKey: ContentMotion.opacityKey) as? CABasicAnimation)
			}
		}
		await harness.playground.supersede(oldId)
		harness.inbox.requestRead()
		try await waitUntil("the row takes its successor") { start != nil }
		let newTitle = row.rowContent.message.title
		XCTAssertNotEqual(newTitle, oldTitle)
		XCTAssertEqual(transitionKeys, [], "the header fades on the motion clock, not in a transition")
		let changeStart = try XCTUnwrap(start)
		let frameRate = 120.0
		let samples = (0 ... Int((MotionSpec.crossfadeDuration * frameRate).rounded(.up))).map { frame in
			let time = changeStart + Double(frame) / frameRate
			return (old: Self.ink(of: oldTitle, in: inks, at: time), new: Self.ink(of: newTitle, in: inks, at: time))
		}
		XCTAssertEqual(samples.first?.old ?? 0, 1, accuracy: Self.tolerance, "the old title shows whole as the change starts")
		XCTAssertEqual(samples.first?.new ?? 1, 0, accuracy: Self.tolerance)
		XCTAssertEqual(samples.last?.old ?? 1, 0, accuracy: Self.tolerance)
		XCTAssertEqual(samples.last?.new ?? 0, 1, accuracy: Self.tolerance, "the new title shows whole once the fade has run")
		let steepestStep = 4.0451 / (frameRate * MotionSpec.crossfadeDuration) + Self.tolerance
		for (index, sample) in samples.enumerated() {
			XCTAssertEqual(sample.old + sample.new, 1, accuracy: Self.tolerance, "one title's worth of ink at frame \(index)")
		}
		for (index, (earlier, later)) in zip(samples, samples.dropFirst()).enumerated() {
			XCTAssertLessThanOrEqual(abs(later.new - earlier.new), steepestStep, "the new title jumps at frame \(index + 1)")
		}
	}

	// MARK: Sections, the list's scroll, the end of a motion

	func testTheSectionsStackWithNoGapAndTheListHoldsItsRows() async throws {
		let harness = try await makeHarness()
		try await populate(harness)
		harness.content.settle()
		let slots = harness.sectionSlots
		XCTAssertEqual(slots.first?.frame.minY ?? -1, 0, accuracy: Self.tolerance)
		for (upper, lower) in zip(slots, slots.dropFirst()) {
			XCTAssertEqual(lower.frame.minY, upper.frame.maxY, accuracy: Self.tolerance, "no gap or overlap")
		}
		XCTAssertEqual(cardChanges.last, .settled(height: slots.last?.frame.maxY ?? 0))
		let listSlot = try XCTUnwrap(harness.listSlot)
		let scroller = try XCTUnwrap(harness.scroller)
		let rowsHeight = harness.rows.reduce(0) { $0 + $1.frame.height }
		XCTAssertFalse(harness.content.isCapped)
		XCTAssertEqual(listSlot.frame.height, rowsHeight, accuracy: Self.tolerance, "the list is as tall as its rows")
		XCTAssertEqual(scroller.frame.height, rowsHeight, accuracy: Self.tolerance)
		XCTAssertEqual(listSlot.content.frame, scroller.frame)
	}

	func testACappedListScrollsWithinTheCard() async throws {
		let maxHeight: CGFloat = 260
		let harness = try await makeHarness(maxHeight: maxHeight)
		try await populate(harness)
		harness.content.settle()
		XCTAssertTrue(harness.content.isCapped)
		XCTAssertGreaterThan(harness.content.naturalHeight, maxHeight)
		XCTAssertEqual(cardChanges.last, .settled(height: maxHeight), "the card stops at the cap")
		let listSlot = try XCTUnwrap(harness.listSlot)
		let scroller = try XCTUnwrap(harness.scroller)
		let document = try XCTUnwrap(scroller.documentView)
		let rowsHeight = harness.rows.reduce(0) { $0 + $1.frame.height }
		XCTAssertEqual(scroller.frame.height, listSlot.frame.height, accuracy: Self.tolerance, "the scroll view fills the list section")
		XCTAssertLessThan(scroller.frame.height, rowsHeight)
		XCTAssertGreaterThanOrEqual(document.frame.height, rowsHeight, "the document holds every row")
	}

	func testTheCardIsToldOnceEveryMotionHasRun() async throws {
		let harness = try await makeHarness()
		cardChanges.removeAll()
		try await populate(harness)
		let listMotion = try XCTUnwrap(harness.listSlot?.contentMotion)
		XCTAssertTrue(cardChanges.contains { if case .moving = $0 { true } else { false } }, "the card moves")
		harness.clock.advance(to: listMotion.start + Tokens.motionDuration / 2)
		XCTAssertNotEqual(cardChanges.last, .motionEnded, "not halfway")
		harness.clock.advance(to: listMotion.start + Tokens.motionDuration)
		XCTAssertEqual(cardChanges.last, .motionEnded)
		let shownList = try XCTUnwrap(harness.listSlot).shownContent(at: harness.clock.now)
		XCTAssertEqual(shownList.opacity, 1, "the list shows whole")
		XCTAssertEqual(shownList.scale, 1, accuracy: Self.tolerance)
	}

	// MARK: Helpers

	/// A content view on a playground inbox, read once and showing it
	/// empty, its motion on a hand-wound clock: off screen, Core Animation
	/// ends every motion as it is committed. Zooms run whatever the Mac's
	/// Reduce Motion.
	private func makeHarness(maxHeight: CGFloat = .greatestFiniteMagnitude) async throws -> Harness {
		_ = NSApplication.shared
		StatusItemController.reduceMotionOverride = false
		let clock = ManualClock(at: Self.origin)
		let playground = PlaygroundInbox()
		let inbox = InboxModel(service: playground)
		let app = AppModel()
		let content = InboxContentView(app: app, clock: clock)
		content.maxHeight = maxHeight
		content.onCardChange = { [weak self] change in
			self?.cardChanges.append(change)
			self?.onCardChange?(change)
		}
		let window = ShownWindow(
			contentRect: NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: 800), styleMask: [.borderless],
			backing: .buffered, defer: true,
		)
		window.isReleasedWhenClosed = false
		window.contentView?.addSubview(content)
		let harness = Harness(clock: clock, playground: playground, inbox: inbox, content: content, window: window)
		app.run(inbox)
		try await waitUntil("the inbox is read") { inbox.inbox != nil }
		content.settle()
		addTeardownBlock { @MainActor in
			window.contentView = nil
			StatusItemController.reduceMotionOverride = nil
		}
		return harness
	}

	/// Fills the playground and waits until the list's change has started.
	private func populate(_ harness: Harness) async throws {
		await harness.playground.populate()
		harness.inbox.requestRead()
		try await waitUntil("the list comes in") { !harness.rows.isEmpty && harness.listSlot?.contentMotion != nil }
	}

	/// The most any field showing `text` shows at `time`, as the render
	/// server draws it: held at the fade's start before it begins, the model
	/// once it ends.
	private static func ink(of text: String, in inks: [Ink], at time: TimeInterval) -> CGFloat {
		inks.filter { $0.text == text }.map { ink in
			guard let fade = ink.fade, let from = (fade.fromValue as? NSNumber)?.doubleValue,
			      let to = (fade.toValue as? NSNumber)?.doubleValue
			else { return ink.opacity }
			let progress = (time - fade.beginTime) / fade.duration
			if progress < 0 { return CGFloat(from) }
			guard progress < 1 else { return ink.opacity }
			return CGFloat(from + (to - from) * MotionCurve.standard.value(at: progress))
		}.max() ?? 0
	}

	private static func descendants<View: NSView>(of view: NSView) -> [View] {
		view.subviews.flatMap { subview in
			((subview as? View).map { [$0] } ?? []) + descendants(of: subview)
		}
	}

	private static func layers(in view: NSView) -> [CALayer] {
		([view] + descendants(of: view)).compactMap(\.layer)
	}

	private static func hasMoreButton(_ view: NSView) -> Bool {
		view.subviews.contains { subview in
			(subview is NSButton && subview.accessibilityIdentifier().hasPrefix("message-menu:")) || hasMoreButton(subview)
		}
	}

	private func waitUntil(_ what: String, _ condition: @MainActor () -> Bool) async throws {
		let deadline = Date().addingTimeInterval(Self.timeout)
		while !condition() {
			guard Date() < deadline else {
				XCTFail("timed out: \(what)")
				throw CancellationError()
			}
			try await Task.sleep(for: .milliseconds(2))
		}
	}
}
