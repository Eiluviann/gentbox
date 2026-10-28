import AppKit
import QuartzCore
import XCTest

/// The views carry the motion frame by frame: each slot as the render
/// server draws it, its model frame plus the additive animations read back
/// off its layer, sampled at 120 Hz through bursts of changes landing
/// mid-motion, on a hand-wound clock.
@MainActor
final class InboxTransitionTests: XCTestCase {
	private typealias Stack = SlotStack<String>
	private typealias Slot = StackPlan<String>.Slot
	private typealias Frame = StackPlan<String>.Frame

	private final class ManualClock: MotionClock {
		private(set) var now: TimeInterval
		private var pending: [(time: TimeInterval, work: @MainActor () -> Void)] = []

		init(at time: TimeInterval) {
			now = time
		}

		func whenEnded(at time: TimeInterval, _ work: @escaping @MainActor () -> Void) {
			pending.append((time, work))
		}

		/// Runs every completion due by `time`, earliest first, those
		/// scheduled first first at a tie.
		func advance(to time: TimeInterval) {
			while let next = pending.indices.min(by: { pending[$0].time < pending[$1].time }), pending[next].time <= time {
				let due = pending.remove(at: next)
				now = due.time
				due.work()
			}
			now = time
		}
	}

	private struct Change {
		/// After the playback's start.
		let time: TimeInterval
		let listed: [Slot]
	}

	private struct Track: Hashable {
		let key: String
		let delta: Double
		let begin: TimeInterval
		let duration: TimeInterval
	}

	private struct Sample {
		let time: TimeInterval
		let order: [String]
		/// As drawn: the views' frames and their layers' animations.
		let frames: [String: Frame]
		/// As the motion model has it.
		let model: [String: Frame]
		let raised: Set<String>
		let zPositions: [String: CGFloat]
		let topTracks: [String: [Track]]
		let heightTracks: [String: [Track]]
	}

	private struct Playback {
		let stack: Stack
		let views: [String: SlotView]
		let samples: [Sample]
		let dropped: [String]
	}

	private static let frameRate = 120.0
	/// The curve's steepest slope (`MotionTimelineTests`).
	private static let steepestSlope = 4.0451
	private static let tolerance: CGFloat = 1e-6
	private static let width: CGFloat = 380
	private static let rowHeight: CGFloat = 60
	private static let openHeight: CGFloat = 200
	/// Far from 0, which Core Animation reads as "now".
	private static let origin: TimeInterval = 1000

	private var duration: TimeInterval {
		Tokens.motionDuration
	}

	// MARK: Geometry, frame by frame

	func testAChangeDrawsEveryFrameTheModelDoes() {
		let playback = play(from: slots("a", "b", "c"), [Change(time: 0, listed: slots("a", "x", "c"))])
		assertDrawsTheModel(playback)
		assertGapless(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: slots("a", "x", "c"))
		XCTAssertEqual(playback.dropped, ["b"])
	}

	/// b, taken back before it has left, returns to its place.
	func testABurstOfInsertsAndRemovesKeepsItsOrder() {
		let changes = [
			Change(time: 0, listed: slots("a", "x", "c", "d")),
			Change(time: 0.05, listed: slots("y", "a", "x", "c")),
			Change(time: 0.1, listed: slots("y", "a", "x", "z", "c")),
			Change(time: 0.12, listed: slots("y", "x", "z", "c")),
			Change(time: 0.2, listed: slots("y", "b", "x", "z", "c")),
		]
		let playback = play(from: slots("a", "b", "c", "d"), changes)
		assertDrawsTheModel(playback)
		assertGapless(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: slots("y", "b", "x", "z", "c"))
		XCTAssertEqual(playback.dropped.sorted(), ["a", "d"], "each leaves once its change ends")
	}

	/// b, taken back elsewhere before it has left, slides there above the
	/// others, which close its place behind it.
	func testABurstThatTakesARowBackElsewhereCoversNothingButTheMover() {
		let changes = [
			Change(time: 0, listed: slots("a", "x", "c", "d")),
			Change(time: 0.05, listed: slots("y", "a", "x", "c")),
			Change(time: 0.12, listed: slots("y", "x", "c")),
			Change(time: 0.2, listed: slots("y", "x", "c", "b")),
		]
		let playback = play(from: slots("a", "b", "c", "d"), changes)
		assertDrawsTheModel(playback)
		assertNoOverlap(playback)
		assertNoJumps(playback)
		XCTAssertTrue(playback.samples.contains { $0.raised == ["b"] }, "b moves")
		assertSettled(playback, on: slots("y", "x", "c", "b"))
		XCTAssertEqual(playback.dropped.sorted(), ["a", "d"], "each leaves once its change ends")
	}

	func testTheOpenCardHandsOverWithoutGapsOrJumps() {
		let changes = [
			Change(time: 0, listed: slots("a", "b", "c", open: "b")),
			Change(time: 0.1, listed: slots("a", "b", "c", open: "c")),
			Change(time: 0.15, listed: slots("a", "c", open: "c")),
		]
		let playback = play(from: slots("a", "b", "c", open: "a"), changes)
		assertDrawsTheModel(playback)
		assertGapless(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: slots("a", "c", open: "c"))
	}

	/// A row that moves slides above the rows it passes, drawn over them
	/// until its change ends, and none of the others covers another.
	func testAMoverSlidesAboveTheOthersUntilItsChangeEnds() {
		let changes = [
			Change(time: 0, listed: slots("a", "b", "c", "d")),
			Change(time: 0.1, listed: slots("a", "e", "b", "c", "d")),
		]
		let playback = play(from: slots("d", "a", "b", "c"), changes)
		assertDrawsTheModel(playback)
		assertNoOverlap(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: slots("a", "e", "b", "c", "d"))
		let midway = playback.samples.filter { $0.time > Self.origin && $0.time < Self.origin + duration }
		XCTAssertFalse(midway.isEmpty)
		for sample in midway {
			XCTAssertEqual(sample.raised, ["d"], "at \(sample.time)")
			let others = sample.zPositions.filter { $0.key != "d" }.values
			XCTAssertGreaterThan(sample.zPositions["d"] ?? 0, others.max() ?? 0, "drawn over the rest at \(sample.time)")
		}
	}

	func testARowTakenBackAsItLeavesComesBackFromWhereItShows() {
		let changes = [
			Change(time: 0, listed: slots("a", "c")),
			Change(time: 0.06, listed: slots("a", "b", "c")),
		]
		let playback = play(from: slots("a", "b", "c"), changes)
		assertDrawsTheModel(playback)
		assertGapless(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: slots("a", "b", "c"))
		XCTAssertTrue(playback.dropped.isEmpty)
		let fade = basicAnimation(playback.views["b"]?.content.layer, ContentMotion.opacityKey)
		let shown = 1 - Float(MotionCurve.standard.value(at: 0.06 / Tokens.despawnDuration))
		XCTAssertEqual((fade?.fromValue as? NSNumber)?.floatValue ?? -1, shown, accuracy: 1e-6, "from where its fade-out had got to")
		XCTAssertEqual((fade?.toValue as? NSNumber)?.floatValue, 1)
		XCTAssertEqual(fade?.duration ?? 0, Tokens.spawnDuration, accuracy: 1e-9)
		XCTAssertFalse(playback.views["b"]?.isLeaving ?? true)
	}

	// MARK: Content

	func testArrivalsFadeInAndZoomUpAndDeparturesFadeOutAndHoldTheirZoom() throws {
		let (stack, views) = settledStack(slots("a", "b"))
		let arriving = SlotView()
		arriving.content.frame = NSRect(x: 0, y: 0, width: Self.width, height: Self.rowHeight)
		stack.apply(entries(slots("a", "new"), views: views, extra: ["new": arriving]), change: 1, at: Self.origin, duration: duration, reducesMotion: false)

		let fadeIn = try XCTUnwrap(basicAnimation(arriving.content.layer, ContentMotion.opacityKey))
		XCTAssertEqual(fadeIn.fromValue as? Float, 0)
		XCTAssertEqual(fadeIn.toValue as? Float, 1)
		XCTAssertEqual(fadeIn.duration, Tokens.spawnDuration, accuracy: 1e-9)
		XCTAssertEqual(fadeIn.beginTime, Self.origin, accuracy: 1e-9)
		XCTAssertEqual(arriving.content.alphaValue, 1)
		let zoomIn = try XCTUnwrap(basicAnimation(arriving.content.layer, ContentMotion.transformKey))
		assertScale(zoomIn.fromValue, Tokens.arrivalScale, about: arriving.content)
		assertScale(zoomIn.toValue, 1, about: arriving.content)
		XCTAssertTrue(zoomIn.isRemovedOnCompletion)

		let leaving = try XCTUnwrap(views["b"])
		let fadeOut = try XCTUnwrap(basicAnimation(leaving.content.layer, ContentMotion.opacityKey))
		XCTAssertEqual(fadeOut.fromValue as? Float, 1)
		XCTAssertEqual(fadeOut.toValue as? Float, 0)
		XCTAssertEqual(fadeOut.duration, Tokens.despawnDuration, accuracy: 1e-9)
		XCTAssertEqual(leaving.content.alphaValue, 0)
		let zoomOut = try XCTUnwrap(basicAnimation(leaving.content.layer, ContentMotion.transformKey))
		assertScale(zoomOut.toValue, Tokens.arrivalScale, about: leaving.content)
		XCTAssertFalse(zoomOut.isRemovedOnCompletion, "it holds its zoom until it goes")
		XCTAssertEqual(zoomOut.fillMode, .both)
		XCTAssertTrue(leaving.isLeaving)
		XCTAssertNil(leaving.hitTest(NSPoint(x: 10, y: leaving.frame.minY)), "a leaving slot takes no clicks")
	}

	func testReduceMotionKeepsTheFadesAndDropsTheZooms() {
		let (stack, views) = settledStack(slots("a", "b"))
		let arriving = SlotView()
		stack.apply(entries(slots("a", "new"), views: views, extra: ["new": arriving]), change: 1, at: Self.origin, duration: duration, reducesMotion: true)
		for view in [arriving, views["b"]] {
			XCTAssertNotNil(basicAnimation(view?.content.layer, ContentMotion.opacityKey))
			XCTAssertNil(view?.content.layer?.animation(forKey: ContentMotion.transformKey))
		}
	}

	func testADroppedSlotIsHiddenWithItsMotionAndHandedBack() throws {
		let clock = ManualClock(at: Self.origin)
		let transition = InboxTransition(clock: clock)
		let (stack, views) = settledStack(slots("a", "b"))
		transition.add(stack)
		var dropped: [(String, SlotView)] = []
		stack.onDrop = { dropped.append(($0, $1)) }
		transition.run(for: duration) { change, time in
			stack.apply(entries(slots("a"), views: views), change: change, at: time, duration: duration, reducesMotion: false)
		}
		XCTAssertTrue(dropped.isEmpty, "it closes its place first")
		clock.advance(to: Self.origin + duration)
		let view = try XCTUnwrap(views["b"])
		XCTAssertEqual(dropped.map(\.0), ["b"])
		XCTAssertTrue(dropped.first?.1 === view)
		XCTAssertTrue(view.isHidden)
		XCTAssertTrue(motionKeys(view).isEmpty)
		XCTAssertNil(stack.views["b"])
	}

	// MARK: The coordinator

	func testEveryChangeEndsOnceAndTheLastLetsTheGeometryShrink() {
		let clock = ManualClock(at: Self.origin)
		let transition = InboxTransition(clock: clock)
		var shrinks = 0
		transition.onLastCompletion = { shrinks += 1 }
		let first = transition.run(for: duration) { _, _ in }
		clock.advance(to: Self.origin + 0.1)
		transition.run(for: duration) { _, _ in }
		clock.advance(to: Self.origin + duration)
		XCTAssertEqual(shrinks, 0, "a change still runs")
		XCTAssertTrue(transition.isMoving)
		clock.advance(to: Self.origin + 0.1 + duration)
		XCTAssertEqual(shrinks, 1)
		XCTAssertFalse(transition.isMoving)
		transition.complete(first)
		XCTAssertEqual(shrinks, 1, "a completion lands once")
	}

	func testSettlingEndsEveryChangeAndItsCompletions() {
		let clock = ManualClock(at: Self.origin)
		let transition = InboxTransition(clock: clock)
		let (stack, views) = settledStack(slots("a", "b"))
		transition.add(stack)
		var shrinks = 0
		transition.onLastCompletion = { shrinks += 1 }
		transition.run(for: duration) { change, time in
			stack.apply(entries(slots("b", "a", open: "a"), views: views), change: change, at: time, duration: duration, reducesMotion: false)
		}
		transition.settle()
		XCTAssertFalse(transition.isMoving)
		for (id, top, height) in [("b", 0.0, Self.rowHeight), ("a", Self.rowHeight, Self.openHeight)] {
			XCTAssertEqual(views[id]?.frame, NSRect(x: 0, y: top, width: Self.width, height: height), id)
			XCTAssertEqual(views[id].map(motionKeys), [], id)
		}
		clock.advance(to: Self.origin + duration)
		XCTAssertEqual(shrinks, 0, "the settled change's completion does nothing")
	}

	/// A stack that sits out a change counts only the changes it took part
	/// in: the list's rows, a row's own parts.
	func testAStackEndsOnlyTheChangesItTookPartIn() {
		let clock = ManualClock(at: Self.origin)
		let transition = InboxTransition(clock: clock)
		let (stack, views) = settledStack(slots("a", "b"))
		transition.add(stack)
		transition.run(for: duration) { _, _ in }
		clock.advance(to: Self.origin + 0.1)
		transition.run(for: duration) { change, time in
			stack.apply(entries(slots("a"), views: views), change: change, at: time, duration: duration, reducesMotion: false)
		}
		clock.advance(to: Self.origin + duration)
		XCTAssertNotNil(stack.views["b"], "the first change ended, not the one that removed it")
		clock.advance(to: Self.origin + 0.1 + duration)
		XCTAssertNil(stack.views["b"])
	}

	// MARK: Playback

	private func play(from start: [Slot], _ changes: [Change], reducesMotion: Bool = false) -> Playback {
		precondition(zip(changes, changes.dropFirst()).allSatisfy { $0.time < $1.time }, "changes in time order")
		let clock = ManualClock(at: Self.origin)
		let transition = InboxTransition(clock: clock)
		let (stack, settled) = settledStack(start)
		transition.add(stack)
		var views = settled
		var dropped: [String] = []
		stack.onDrop = { id, _ in dropped.append(id) }
		var pending = changes[...]
		var samples: [Sample] = []
		let lastFrame = Int((((changes.last?.time ?? 0) + duration) * Self.frameRate).rounded(.up)) + 1
		for frame in 0 ... lastFrame {
			let time = Self.origin + Double(frame) / Self.frameRate
			while let next = pending.first, Self.origin + next.time <= time {
				// Completions first at a tie: they were scheduled earlier.
				clock.advance(to: Self.origin + next.time)
				let listed = entries(next.listed, views: views)
				for entry in listed {
					views[entry.id] = entry.view
				}
				transition.run(for: duration) { change, at in
					stack.apply(listed, change: change, at: at, duration: duration, reducesMotion: reducesMotion)
				}
				pending = pending.dropFirst()
			}
			clock.advance(to: time)
			samples.append(sample(stack, at: time))
		}
		return Playback(stack: stack, views: views, samples: samples, dropped: dropped)
	}

	private func sample(_ stack: Stack, at time: TimeInterval) -> Sample {
		let order = stack.timeline.order
		var frames: [String: Frame] = [:]
		var model: [String: Frame] = [:]
		var zPositions: [String: CGFloat] = [:]
		var topTracks: [String: [Track]] = [:]
		var heightTracks: [String: [Track]] = [:]
		for id in order {
			guard let view = stack.views[id] else {
				XCTFail("\(id) without a view at \(time)")
				continue
			}
			XCTAssertFalse(view.isHidden, "\(id) hidden while placed at \(time)")
			let tops = tracks(on: view.layer, keyPath: "position.y")
			let heights = tracks(on: view.layer, keyPath: "bounds.size.height")
			frames[id] = Frame(top: view.frame.minY + offset(tops, at: time), height: view.frame.height + offset(heights, at: time))
			model[id] = stack.timeline.frame(of: id, at: time)
			zPositions[id] = view.layer?.zPosition
			topTracks[id] = tops
			heightTracks[id] = heights
		}
		return Sample(
			time: time, order: order, frames: frames, model: model, raised: Set(order.filter(stack.timeline.isRaised)),
			zPositions: zPositions, topTracks: topTracks, heightTracks: heightTracks,
		)
	}

	/// A layer's committed motion on `keyPath`, as the render server runs it.
	private func tracks(on layer: CALayer?, keyPath: String) -> [Track] {
		guard let layer else { return [] }
		return (layer.animationKeys() ?? []).filter { $0.hasPrefix("motion.\(keyPath).") }.compactMap { key in
			guard let animation = basicAnimation(layer, key), let delta = (animation.fromValue as? NSNumber)?.doubleValue else {
				XCTFail("\(key) is no additive basic animation")
				return nil
			}
			XCTAssertEqual(animation.keyPath, keyPath, key)
			XCTAssertTrue(animation.isAdditive, key)
			XCTAssertEqual((animation.toValue as? NSNumber)?.doubleValue, 0, key)
			XCTAssertEqual(animation.fillMode, .backwards, key)
			XCTAssertTrue(animation.isRemovedOnCompletion, key)
			assertTheMotionCurve(animation.timingFunction, key)
			return Track(key: key, delta: delta, begin: animation.beginTime, duration: animation.duration)
		}
	}

	/// How far from its model the render server draws a value at `time`:
	/// held at the start before the motion begins, gone once it ends.
	private func offset(_ tracks: [Track], at time: TimeInterval) -> CGFloat {
		tracks.reduce(0) { sum, track in
			let progress = (time - track.begin) / track.duration
			guard progress < 1 else { return sum }
			return sum + CGFloat(track.delta * (1 - MotionCurve.standard.value(at: progress)))
		}
	}

	// MARK: Checks

	private func assertDrawsTheModel(_ playback: Playback, file: StaticString = #filePath, line: UInt = #line) {
		for sample in playback.samples {
			for id in sample.order {
				assertEqual(sample.frames[id], sample.model[id], "\(id) at \(sample.time)", file: file, line: line)
			}
		}
	}

	/// No slot covers another unless it is raised, and none shows below no
	/// height.
	private func assertNoOverlap(_ playback: Playback, file: StaticString = #filePath, line: UInt = #line) {
		for sample in playback.samples {
			var bottom: CGFloat = 0
			for id in sample.order {
				guard let frame = sample.frames[id] else { return XCTFail("\(id) without a frame", file: file, line: line) }
				let moment = "\(id) at \(sample.time)"
				XCTAssertGreaterThanOrEqual(frame.top, -Self.tolerance, moment, file: file, line: line)
				XCTAssertGreaterThanOrEqual(frame.height, -Self.tolerance, moment, file: file, line: line)
				guard !sample.raised.contains(id) else { continue }
				XCTAssertGreaterThanOrEqual(frame.top, bottom - Self.tolerance, "\(moment) covers the slot above", file: file, line: line)
				bottom = frame.bottom
			}
		}
	}

	/// Without movers each slot touches the next, top to bottom, in the
	/// stack's order, at every frame.
	private func assertGapless(_ playback: Playback, file: StaticString = #filePath, line: UInt = #line) {
		assertNoOverlap(playback, file: file, line: line)
		for sample in playback.samples {
			var bottom: CGFloat = 0
			for id in sample.order {
				XCTAssertEqual(sample.frames[id]?.top ?? -1, bottom, accuracy: Self.tolerance, "a gap above \(id) at \(sample.time)", file: file, line: line)
				bottom = sample.frames[id]?.bottom ?? bottom
			}
		}
	}

	/// From one frame to the next no slot moves further than its motions
	/// can at the curve's steepest, changes landing between them included.
	private func assertNoJumps(_ playback: Playback, file: StaticString = #filePath, line: UInt = #line) {
		for (previous, next) in zip(playback.samples, playback.samples.dropFirst()) {
			let interval = next.time - previous.time
			for (id, frame) in next.frames {
				guard let was = previous.frames[id] else {
					XCTAssertEqual(frame.height, 0, accuracy: Self.tolerance, "\(id) opens from no height", file: file, line: line)
					continue
				}
				let topReach = reach(previous.topTracks[id], next.topTracks[id], over: interval)
				let heightReach = reach(previous.heightTracks[id], next.heightTracks[id], over: interval)
				XCTAssertLessThanOrEqual(abs(frame.top - was.top), topReach + Self.tolerance, "\(id)'s top jumps at \(next.time)", file: file, line: line)
				XCTAssertLessThanOrEqual(abs(frame.height - was.height), heightReach + Self.tolerance, "\(id)'s height jumps at \(next.time)", file: file, line: line)
			}
		}
	}

	/// The motion ends on the target: every slot at its place, nothing raised
	/// or moving, drawn in order, and every slot that left hidden.
	private func assertSettled(_ playback: Playback, on listed: [Slot], file: StaticString = #filePath, line: UInt = #line) {
		guard let last = playback.samples.last else { return XCTFail("no frames", file: file, line: line) }
		XCTAssertEqual(last.order, listed.map(\.id), file: file, line: line)
		XCTAssertTrue(last.raised.isEmpty, file: file, line: line)
		var top: CGFloat = 0
		for (index, slot) in listed.enumerated() {
			XCTAssertEqual(playback.views[slot.id]?.frame, NSRect(x: 0, y: top, width: Self.width, height: slot.height), slot.id, file: file, line: line)
			assertEqual(last.frames[slot.id], Frame(top: top, height: slot.height), "\(slot.id) drawn at its place", file: file, line: line)
			XCTAssertEqual(last.zPositions[slot.id], CGFloat(index), slot.id, file: file, line: line)
			XCTAssertEqual(playback.views[slot.id]?.content.alphaValue, 1, slot.id, file: file, line: line)
			top += slot.height
		}
		XCTAssertEqual(playback.stack.targetHeight, top, file: file, line: line)
		let listedIds = Set(listed.map(\.id))
		for (id, view) in playback.views where !listedIds.contains(id) {
			XCTAssertTrue(view.isHidden, "\(id) left", file: file, line: line)
		}
	}

	private func assertEqual(_ frame: Frame?, _ expected: Frame?, _ message: String, file: StaticString, line: UInt) {
		guard let frame, let expected else { return XCTAssertEqual(frame, expected, message, file: file, line: line) }
		XCTAssertEqual(frame.top, expected.top, accuracy: Self.tolerance, message, file: file, line: line)
		XCTAssertEqual(frame.height, expected.height, accuracy: Self.tolerance, message, file: file, line: line)
	}

	private func assertTheMotionCurve(_ timing: CAMediaTimingFunction?, _ message: String, file: StaticString = #filePath, line: UInt = #line) {
		var committed: [Float] = [0, 0]
		var model: [Float] = [0, 0]
		for index in 0 ..< 4 {
			timing?.getControlPoint(at: index, values: &committed)
			MotionCurve.standard.timingFunction.getControlPoint(at: index, values: &model)
			XCTAssertEqual(committed, model, message, file: file, line: line)
		}
	}

	/// A centred scale of `scale` about `view`'s middle.
	private func assertScale(_ value: Any?, _ scale: CGFloat, about view: NSView, file: StaticString = #filePath, line: UInt = #line) {
		guard let transform = (value as? NSValue)?.caTransform3DValue else { return XCTFail("no transform", file: file, line: line) }
		XCTAssertEqual(transform.m11, scale, accuracy: 1e-9, file: file, line: line)
		XCTAssertEqual(transform.m22, scale, accuracy: 1e-9, file: file, line: line)
		XCTAssertEqual(transform.m41, view.bounds.width / 2 * (1 - scale), accuracy: 1e-9, "about its middle", file: file, line: line)
		XCTAssertEqual(transform.m42, view.bounds.height / 2 * (1 - scale), accuracy: 1e-9, "about its middle", file: file, line: line)
	}

	// MARK: Helpers

	/// The furthest a value can move in `interval` under the motions it had
	/// at either end of it.
	private func reach(_ earlier: [Track]?, _ later: [Track]?, over interval: TimeInterval) -> CGFloat {
		let tracks = Set((earlier ?? []) + (later ?? []))
		return tracks.reduce(0) { $0 + CGFloat(abs($1.delta) * Self.steepestSlope * interval / $1.duration) }
	}

	private func settledStack(_ start: [Slot]) -> (Stack, [String: SlotView]) {
		let container = FlippedView(frame: NSRect(x: 0, y: 0, width: Self.width, height: 1000))
		let stack = Stack(in: container, width: Self.width)
		// The container is the stack's to hold, for the test's length.
		containers.append(container)
		let listed = entries(start, views: [:])
		stack.settle(listed)
		return (stack, Dictionary(uniqueKeysWithValues: listed.map { ($0.id, $0.view) }))
	}

	private var containers: [NSView] = []

	private func entries(_ listed: [Slot], views: [String: SlotView], extra: [String: SlotView] = [:]) -> [Stack.Entry] {
		listed.map { slot in
			let view = views[slot.id] ?? extra[slot.id] ?? Self.makeSlot()
			return Stack.Entry(id: slot.id, view: view, height: slot.height)
		}
	}

	private static func makeSlot() -> SlotView {
		let view = SlotView()
		view.content.frame = NSRect(x: 0, y: 0, width: width, height: rowHeight)
		return view
	}

	private func basicAnimation(_ layer: CALayer?, _ key: String) -> CABasicAnimation? {
		layer?.animation(forKey: key) as? CABasicAnimation
	}

	private func motionKeys(_ view: SlotView) -> [String] {
		[view.layer, view.content.layer].flatMap { $0?.animationKeys() ?? [] }.filter { $0.hasPrefix("motion.") }
	}

	/// Rows, `open` at the open card's height.
	private func slots(_ ids: String..., open: String? = nil) -> [Slot] {
		ids.map { Slot(id: $0, height: $0 == open ? Self.openHeight : Self.rowHeight) }
	}
}
