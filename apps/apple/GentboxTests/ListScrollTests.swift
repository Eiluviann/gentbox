import AppKit
import QuartzCore
import XCTest

/// Where the list scrolls to for a change (ADR 0013, Anchor), and the
/// scroll as drawn frame by frame at 120 Hz: the document's slide and the
/// rows' motion together keep the anchor still on screen, through bursts
/// of changes and a scroll of the user's that cuts the slide short.
@MainActor
final class ListScrollTests: XCTestCase {
	private typealias Timeline = MotionTimeline<String>
	private typealias Slot = Timeline.Slot
	private typealias Stack = SlotStack<String>

	private struct Change {
		/// After the playback's start.
		let time: TimeInterval
		let listed: [Slot]
	}

	private struct Track {
		let delta: Double
		let begin: TimeInterval
		let duration: TimeInterval
	}

	private static let frameRate = 120.0
	/// The curve's steepest slope (`MotionTimelineTests`).
	private static let steepestSlope = 4.0451
	private static let tolerance: CGFloat = 1e-6
	private static let width: CGFloat = 380
	private static let viewHeight: CGFloat = 300
	private static let rowHeight: CGFloat = 60
	private static let openHeight: CGFloat = 200
	/// Far from 0, which Core Animation reads as "now".
	private static let origin: TimeInterval = 1000

	private var duration: TimeInterval {
		Tokens.motionDuration
	}

	// MARK: The target

	func testAtTheTopNoRowAnchorsTheList() {
		let timeline = settled(rows(0 ..< 10))
		XCTAssertNil(ListScrollTarget.anchor(in: timeline, origin: 0))
		let inserted = settled([Slot(id: "new", height: Self.rowHeight)] + rows(0 ..< 10))
		let target = ListScrollTarget.origin(from: 0, anchor: nil, in: inserted, showing: nil, viewHeight: Self.viewHeight)
		XCTAssertEqual(target, 0, "a row inserted at the top shows")
	}

	func testTheFirstRowReachingBelowTheTopAnchorsTheList() throws {
		let timeline = settled(rows(0 ..< 10))
		let midRow = try XCTUnwrap(ListScrollTarget.anchor(in: timeline, origin: 90))
		XCTAssertEqual(midRow.id, "r1")
		XCTAssertEqual(midRow.top, 60)
		let atEdge = try XCTUnwrap(ListScrollTarget.anchor(in: timeline, origin: 120))
		XCTAssertEqual(atEdge.id, "r2", "a row ending at the top is out of view")
	}

	func testRowsOpeningOrClosingAboveTheAnchorLeaveItWhereItShows() throws {
		let before = settled(rows(0 ..< 10))
		let anchor = try XCTUnwrap(ListScrollTarget.anchor(in: before, origin: 130))
		let opened = settled([Slot(id: "r0", height: Self.openHeight)] + rows(1 ..< 10))
		let openedTarget = ListScrollTarget.origin(from: 130, anchor: anchor, in: opened, showing: nil, viewHeight: Self.viewHeight)
		XCTAssertEqual(openedTarget, 270)
		let removed = settled(rows(0 ..< 1) + rows(2 ..< 10))
		let removedTarget = ListScrollTarget.origin(from: 130, anchor: anchor, in: removed, showing: nil, viewHeight: Self.viewHeight)
		XCTAssertEqual(removedTarget, 70)
	}

	func testANewOpenCardShowsWholeByTheLeastScroll() {
		let below = settled(rows(0 ..< 6) + [Slot(id: "open", height: Self.openHeight)] + rows(6 ..< 10))
		XCTAssertEqual(
			ListScrollTarget.origin(from: 0, anchor: nil, in: below, showing: "open", viewHeight: Self.viewHeight), 260,
			"its bottom at the view's bottom",
		)
		let tall = settled(rows(0 ..< 6) + [Slot(id: "open", height: 400)] + rows(6 ..< 10))
		XCTAssertEqual(
			ListScrollTarget.origin(from: 0, anchor: nil, in: tall, showing: "open", viewHeight: Self.viewHeight), 360,
			"a card taller than the view shows its top",
		)
		let above = settled(rows(0 ..< 2) + [Slot(id: "open", height: Self.openHeight)] + rows(2 ..< 10))
		XCTAssertEqual(
			ListScrollTarget.origin(from: 300, anchor: nil, in: above, showing: "open", viewHeight: Self.viewHeight), 120,
			"its top at the view's top",
		)
		XCTAssertEqual(
			ListScrollTarget.origin(from: 100, anchor: nil, in: above, showing: "open", viewHeight: Self.viewHeight), 100,
			"a card already whole stays",
		)
	}

	func testTheTargetStaysWithinTheDocument() throws {
		let before = settled(rows(0 ..< 10))
		let anchor = try XCTUnwrap(ListScrollTarget.anchor(in: before, origin: 290))
		let shorter = settled(rows(0 ..< 6))
		XCTAssertEqual(
			ListScrollTarget.origin(from: 290, anchor: anchor, in: shorter, showing: nil, viewHeight: Self.viewHeight), 60,
		)
		let short = settled(rows(0 ..< 2))
		XCTAssertEqual(ListScrollTarget.origin(from: 290, anchor: anchor, in: short, showing: nil, viewHeight: Self.viewHeight), 0)
	}

	func testAScrollPastTheEndSlidesBackButNotARoundingOfIt() {
		let lastOrigin = Self.rowHeight * 10 - Self.viewHeight
		let rowsHeight = Self.rowHeight * 10
		XCTAssertEqual(
			ListScrollTarget.slideBack(from: lastOrigin + 40, rowsHeight: rowsHeight, viewHeight: Self.viewHeight), lastOrigin,
		)
		XCTAssertNil(ListScrollTarget.slideBack(from: lastOrigin, rowsHeight: rowsHeight, viewHeight: Self.viewHeight))
		XCTAssertNil(
			ListScrollTarget.slideBack(from: lastOrigin + 0.3, rowsHeight: rowsHeight, viewHeight: Self.viewHeight),
			"a rounding of the scroll is no slide",
		)
		XCTAssertEqual(
			ListScrollTarget.slideBack(from: 40, rowsHeight: Self.rowHeight * 2, viewHeight: Self.viewHeight), 0,
			"a list shorter than its view ends at the top",
		)
	}

	// MARK: The scroll, frame by frame

	func testTheAnchorStaysStillOnScreenThroughABurst() {
		let list = List(start: rows(0 ..< 10), origin: 130, duration: duration)
		let changes = [
			Change(time: 0, listed: [Slot(id: "r0", height: Self.openHeight)] + rows(1 ..< 10)),
			Change(time: 0.1, listed: [Slot(id: "r0", height: Self.openHeight)] + rows(2 ..< 10)),
			Change(time: 0.15, listed: [Slot(id: "new", height: Self.rowHeight), Slot(id: "r0", height: Self.openHeight)] + rows(2 ..< 10)),
		]
		var landed = 0
		for time in frames(through: (changes.last?.time ?? 0) + duration) {
			while let next = changes[safe: landed], next.time <= time {
				list.apply(next.listed, at: Self.origin + next.time)
				landed += 1
			}
			list.clock.advance(to: Self.origin + time)
			let shown = list.shownOrigin(at: Self.origin + time)
			XCTAssertEqual(shown, list.scroller.shownOrigin(at: Self.origin + time), accuracy: Self.tolerance, "drawn as modelled")
			XCTAssertEqual(list.shownTop(of: "r2", at: Self.origin + time) - shown, -10, accuracy: 1e-4, "r2 at \(time)")
		}
		let end = Self.origin + (changes.last?.time ?? 0) + duration
		XCTAssertEqual(list.scroller.origin, 270)
		XCTAssertEqual(list.shownOrigin(at: end), 270, accuracy: 1e-4, "the slides end on the target")
	}

	func testAScrollOfTheUsersTakesTheOffsetAsItShows() throws {
		let list = List(start: rows(0 ..< 10), origin: 130, duration: duration)
		list.apply([Slot(id: "r0", height: Self.openHeight)] + rows(1 ..< 10), at: Self.origin)
		let time = Self.origin + 0.1
		list.clock.advance(to: time)
		list.scroller.now = { time }
		let shown = list.shownOrigin(at: time)
		let anchorOnScreen = list.shownTop(of: "r2", at: time) - shown
		let event = try XCTUnwrap(
			CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: 0, wheel2: 0, wheel3: 0).flatMap(NSEvent.init(cgEvent:)),
		)
		list.scroller.scrollWheel(with: event)
		XCTAssertEqual(list.scroller.origin, shown, accuracy: Self.tolerance, "the clip view at the offset as it showed")
		XCTAssertEqual(list.shownOrigin(at: time), shown, accuracy: Self.tolerance, "no jump")
		XCTAssertTrue(motionKeys(list.document).isEmpty, "the slide ended")
		XCTAssertGreaterThanOrEqual(list.document.frame.height, shown + Self.viewHeight, "the document holds the offset")
		XCTAssertEqual(anchorOnScreen, -10, accuracy: 1e-4, "still until the user scrolled")
		var previous = anchorOnScreen
		for frame in frames(through: duration).map({ time + $0 }) where frame <= Self.origin + duration {
			let onScreen = list.shownTop(of: "r2", at: frame) - list.shownOrigin(at: frame)
			XCTAssertGreaterThanOrEqual(onScreen, previous - Self.tolerance, "the rows keep moving: r2 goes on down as r0 opens")
			XCTAssertLessThanOrEqual(onScreen - previous, Self.maxStep(140, over: duration), "no jump at \(frame)")
			previous = onScreen
		}
		XCTAssertEqual(list.shownTop(of: "r2", at: Self.origin + duration) - list.shownOrigin(at: Self.origin + duration), 260 - shown, accuracy: 1e-4)
	}

	// MARK: Helpers

	/// The list as the content view drives it: the row stack in the scroll
	/// view's document, a change capturing the offset and anchor before any
	/// frame changes, the document grown to hold the tallest the rows get and
	/// the target, then the scroll.
	@MainActor
	private final class List {
		let clock: ManualClock
		let scroller = ListScrollView(frame: NSRect(x: 0, y: 0, width: ListScrollTests.width, height: ListScrollTests.viewHeight))
		let document = FlippedView(frame: NSRect(x: 0, y: 0, width: ListScrollTests.width, height: 0))
		let stack: Stack
		private let transition: InboxTransition
		private let duration: TimeInterval
		private var views: [String: SlotView] = [:]

		init(start: [Slot], origin: CGFloat, duration: TimeInterval) {
			clock = ManualClock(at: ListScrollTests.origin)
			transition = InboxTransition(clock: clock)
			self.duration = duration
			scroller.now = { [clock] in clock.now }
			scroller.wantsLayer = true
			scroller.documentView = document
			stack = Stack(in: document, width: ListScrollTests.width)
			transition.add(stack)
			transition.add(scroller)
			stack.settle(entries(start))
			document.setFrameSize(NSSize(width: ListScrollTests.width, height: stack.targetHeight))
			scroller.scroll(from: 0, to: origin, as: nil)
		}

		func apply(_ listed: [Slot], at time: TimeInterval) {
			clock.advance(to: time)
			let previousOrigin = scroller.origin
			let anchor = ListScrollTarget.anchor(in: stack.timeline, origin: previousOrigin)
			let listedEntries = entries(listed)
			transition.run(for: duration) { [self] change, at in
				let motion = MotionChange(change: change, time: at, duration: duration, reducesMotion: false)
				stack.apply(listedEntries, change: change, at: at, duration: duration, reducesMotion: false)
				let target = ListScrollTarget.origin(
					from: previousOrigin, anchor: anchor, in: stack.timeline, showing: nil, viewHeight: ListScrollTests.viewHeight,
				)
				let rowsHeight = max(stack.timeline.tallest(from: at), stack.targetHeight)
				document.setFrameSize(NSSize(width: ListScrollTests.width, height: max(rowsHeight, target + ListScrollTests.viewHeight)))
				scroller.scroll(from: previousOrigin, to: target, as: motion)
			}
		}

		/// The offset the render server draws at `time`.
		func shownOrigin(at time: TimeInterval) -> CGFloat {
			scroller.origin - ListScrollTests.offset(ListScrollTests.tracks(on: document.layer, keyPath: "motion.scroll."), at: time)
		}

		/// A row's top in the document as the render server draws it.
		func shownTop(of id: String, at time: TimeInterval) -> CGFloat {
			guard let view = stack.views[id] else { return .nan }
			return view.frame.minY + ListScrollTests.offset(ListScrollTests.tracks(on: view.layer, keyPath: "motion.position.y."), at: time)
		}

		private func entries(_ listed: [Slot]) -> [Stack.Entry] {
			listed.map { slot in
				let view = views[slot.id] ?? SlotView()
				views[slot.id] = view
				return Stack.Entry(id: slot.id, view: view, height: slot.height)
			}
		}
	}

	private static func tracks(on layer: CALayer?, keyPath prefix: String) -> [Track] {
		guard let layer else { return [] }
		return (layer.animationKeys() ?? []).filter { $0.hasPrefix(prefix) }.compactMap { key in
			guard let animation = layer.animation(forKey: key) as? CABasicAnimation, animation.isAdditive,
			      let delta = (animation.fromValue as? NSNumber)?.doubleValue
			else { return nil }
			return Track(delta: delta, begin: animation.beginTime, duration: animation.duration)
		}
	}

	private static func offset(_ tracks: [Track], at time: TimeInterval) -> CGFloat {
		tracks.reduce(0) { sum, track in
			let progress = (time - track.begin) / track.duration
			guard progress < 1 else { return sum }
			return sum + CGFloat(track.delta * (1 - MotionCurve.standard.value(at: progress)))
		}
	}

	/// The most a value moved by `delta` over `duration` moves in a frame.
	private static func maxStep(_ delta: CGFloat, over duration: TimeInterval) -> CGFloat {
		abs(delta) * steepestSlope / (frameRate * duration) + tolerance
	}

	private func motionKeys(_ view: NSView) -> [String] {
		(view.layer?.animationKeys() ?? []).filter { $0.hasPrefix("motion.") }
	}

	/// Every frame's time at 120 Hz from 0 through `end`.
	private func frames(through end: TimeInterval) -> [TimeInterval] {
		(0 ... Int((end * Self.frameRate).rounded(.up))).map { Double($0) / Self.frameRate }
	}

	private func settled(_ slots: [Slot]) -> Timeline {
		var timeline = Timeline()
		_ = timeline.settle(slots)
		return timeline
	}

	private func rows(_ range: Range<Int>) -> [Slot] {
		range.map { Slot(id: "r\($0)", height: Self.rowHeight) }
	}
}

private extension Array {
	subscript(safe index: Int) -> Element? {
		indices.contains(index) ? self[index] : nil
	}
}
