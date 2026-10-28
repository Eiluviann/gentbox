import CoreGraphics
import QuartzCore
import XCTest

/// The motion model frame by frame at 120 Hz: every change ends on its
/// target stack, nothing overlaps but a raised mover, nothing jumps between
/// frames or as a change lands, and bursts of changes keep the order.
final class MotionTimelineTests: XCTestCase {
	private typealias Timeline = MotionTimeline<String>
	private typealias Slot = Timeline.Slot
	private typealias Frame = Timeline.Frame

	private struct Change {
		let time: TimeInterval
		let listed: [Slot]
	}

	private enum Event {
		case apply([Slot])
		case complete(Int)
	}

	/// What the stack shows at one frame.
	private struct Sample {
		let time: TimeInterval
		/// How many changes and completions had landed.
		let landed: Int
		let order: [String]
		let frames: [String: Frame]
		let raised: Set<String>
		let height: CGFloat
		let tallest: CGFloat
		let topTracks: [String: [MotionTrack]]
		let heightTracks: [String: [MotionTrack]]
		let stackTracks: [MotionTrack]
	}

	private struct Playback {
		let timeline: Timeline
		let samples: [Sample]
		let changes: [Timeline.Change]
		let completions: [Timeline.Completion]
	}

	private static let frameRate = 120.0
	private static let duration = 0.3
	/// The curve's steepest slope, about a third of the way: no value moves
	/// faster than this many times its average speed.
	private static let steepestSlope = 4.0451
	private static let tolerance: CGFloat = 1e-6
	private static let rowHeight: CGFloat = 60
	private static let openHeight: CGFloat = 200

	func testTheCurveRisesFromNothingToAll() {
		let curve = MotionCurve.standard
		XCTAssertEqual(curve.value(at: 0), 0)
		XCTAssertEqual(curve.value(at: 1), 1)
		XCTAssertEqual(curve.value(at: -0.5), 0)
		XCTAssertEqual(curve.value(at: 1.5), 1)
		let values = (0 ... 10000).map { curve.value(at: Double($0) / 10000) }
		let steps = zip(values, values.dropFirst()).map { $1 - $0 }
		XCTAssertTrue(steps.allSatisfy { $0 >= 0 }, "it only rises")
		XCTAssertEqual((steps.max() ?? 0) * 10000, Self.steepestSlope, accuracy: 0.001)
	}

	func testTheCurveIsTheOneCoreAnimationDraws() {
		var points: [Float] = []
		for index in 0 ..< 4 {
			var point: [Float] = [0, 0]
			MotionCurve.standard.timingFunction.getControlPoint(at: index, values: &point)
			points += point
		}
		XCTAssertEqual(points, [0, 0, 0.2, 0, 0, 1, 1, 1])
	}

	func testAChangeEndsOnItsTargetStackWithoutGapsOrJumps() {
		let target = stack("a", "c", "d", open: "c")
		let playback = play(from: stack("a", "b", "c", open: "a"), [Change(time: 0, listed: target)])
		assertGapless(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: target)
		XCTAssertEqual(playback.changes.first?.removed, ["b"])
		XCTAssertEqual(playback.changes.first?.inserted, ["d"])
		XCTAssertEqual(playback.changes.first?.resized, ["a", "c"])
		XCTAssertEqual(playback.completions.first?.dropped, ["b"])
	}

	/// The open card is dismissed and the next row opens in its place.
	func testTheOpenCardHandsOverWithoutGapsOrJumps() {
		let target = stack("b", "c", open: "b")
		let playback = play(from: stack("a", "b", "c", open: "a"), [Change(time: 0, listed: target)])
		assertGapless(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: target)
	}

	/// Changes landing every few frames, mid-motion, each inserting and
	/// removing, one taking back a row still leaving.
	func testABurstOfInsertsAndRemovesKeepsItsOrder() {
		let target = stack("c", "new2", "new3", "new4", open: "c")
		let playback = play(from: stack("a", "b", "c", "d", open: "a"), [
			Change(time: 0, listed: stack("new1", "a", "b", "c", "d", open: "new1")),
			Change(time: 0.04, listed: stack("new1", "a", "c", "d", "new2", open: "new1")),
			Change(time: 0.09, listed: stack("new1", "c", "d", "new2", "new3", open: "new1")),
			Change(time: 0.15, listed: stack("new1", "b", "c", "new2", "new3", open: "c")),
			Change(time: 0.2, listed: target),
		])
		assertGapless(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: target)
		XCTAssertEqual(playback.changes[safe: 3]?.returned, ["b"])
		XCTAssertEqual(playback.changes.flatMap(\.moved), [])
	}

	/// A row inserted under the card as it closes rides the card's bottom.
	func testARowInsertedUnderAClosingCardRidesItsBottom() {
		let target = stack("a", "x", "b", open: "b")
		let playback = play(from: stack("a", "b", open: "a"), [
			Change(time: 0, listed: stack("a", "b", open: "b")),
			Change(time: 0.1, listed: target),
		])
		assertGapless(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: target)
		let copied = playback.changes[safe: 1]?.topTracks["x"]?.map(\.change)
		XCTAssertEqual(copied, [1], "it takes the card's closing as its own")
	}

	/// The row that loses its agent drops to the bottom over the others, and
	/// a row inserted under it mid-motion rides with it, raised as long as it
	/// is.
	func testAMoverSlidesAboveTheOthersUntilItsChangeEnds() {
		let target = stack("a", "b", "c", "d", "x", open: "a")
		let playback = play(from: stack("d", "a", "b", "c", open: "a"), [
			Change(time: 0, listed: stack("a", "b", "c", "d", open: "a")),
			Change(time: 0.1, listed: target),
		])
		assertNoOverlap(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: target)
		XCTAssertEqual(playback.changes.first?.moved, ["d"])
		XCTAssertEqual(playback.changes[safe: 1]?.moved, [])
		XCTAssertEqual(playback.completions.first?.lowered, ["d", "x"])
		for sample in playback.samples {
			switch sample.time {
			case ..<0.09: XCTAssertEqual(sample.raised, ["d"], "at \(sample.time)")
			case 0.11 ..< 0.29: XCTAssertEqual(sample.raised, ["d", "x"], "at \(sample.time)")
			case 0.31...: XCTAssertEqual(sample.raised, [], "at \(sample.time)")
			default: break
			}
		}
	}

	func testAMoverMovedAgainStaysRaisedPastItsFirstChange() {
		var timeline = Timeline()
		_ = timeline.settle(stack("d", "a", "b", "c"))
		XCTAssertEqual(timeline.apply(stack("a", "b", "c", "d"), at: 0, duration: Self.duration).moved, ["d"])
		XCTAssertEqual(timeline.apply(stack("d", "a", "b", "c"), at: 0.1, duration: Self.duration).moved, ["d"])
		XCTAssertEqual(timeline.complete(1).lowered, [], "the second move holds it up")
		XCTAssertTrue(timeline.isRaised("d"))
		XCTAssertEqual(timeline.complete(2).lowered, ["d"])
		XCTAssertFalse(timeline.isRaised("d"))
	}

	func testARowTakenBackAsItLeavesReturnsFromWhereItShows() {
		let target = stack("a", "b", "c")
		let playback = play(from: target, [
			Change(time: 0, listed: stack("a", "c")),
			Change(time: 0.1, listed: target),
		])
		assertGapless(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: target)
		XCTAssertEqual(playback.changes[safe: 1]?.returned, ["b"])
		XCTAssertEqual(playback.changes[safe: 1]?.inserted, [])
		XCTAssertEqual(playback.completions.flatMap(\.dropped), [], "the removal's end no longer ends it")
	}

	/// Back after it left, while the card above still opens: it arrives under
	/// the card's bottom and moves with it.
	func testARowBackAfterItLeftArrivesUnderTheSlotAbove() {
		let target = stack("a", "b", "c", open: "a")
		let playback = play(from: stack("a", "b", "c"), [
			Change(time: 0, listed: stack("a", "c")),
			Change(time: 0.2, listed: stack("a", "c", open: "a")),
			Change(time: 0.35, listed: target),
		])
		assertGapless(playback)
		assertNoJumps(playback)
		assertSettled(playback, on: target)
		XCTAssertEqual(playback.completions.first?.dropped, ["b"])
		XCTAssertEqual(playback.changes[safe: 2]?.inserted, ["b"])
		XCTAssertEqual(playback.changes[safe: 2]?.topTracks["b"]?.map(\.change), [2])
	}

	/// The window holds the tallest frame still to come, so it can grow once
	/// as a change lands and shrink once as it ends.
	func testTheTallestHeightHoldsEveryFrameToCome() {
		let start = stack("a", "b", "c", open: "a")
		let playback = play(from: start, [
			Change(time: 0, listed: stack("a", "b")),
			Change(time: 0.1, listed: stack("a", "b", "c", "d", "e", open: "e")),
		])
		for (index, sample) in playback.samples.enumerated() {
			let toCome = playback.samples[index...].prefix { $0.landed == sample.landed }
			XCTAssertGreaterThanOrEqual(sample.tallest, toCome.map(\.height).max() ?? 0, "at \(sample.time)")
		}
		XCTAssertEqual(playback.samples.first?.tallest ?? 0, 320, accuracy: Self.tolerance, "a shrink holds where it starts")
		XCTAssertEqual(playback.samples.last?.tallest ?? 0, 440, accuracy: Self.tolerance)
	}

	/// A growth overtaken by a shrink peaks between frames: the tallest
	/// still holds the peak, half a point at most over it.
	func testTheTallestHoldsThePeakOfAGrowthOvertakenByAShrink() {
		var timeline = Timeline()
		_ = timeline.settle(stack("a", "b"))
		_ = timeline.apply(stack("a", "b", "c", "d"), at: 0, duration: Self.duration)
		_ = timeline.apply(stack("a"), at: 0.05, duration: Self.duration)
		assertTallestHoldsEveryValue(from: 0, to: 0.35) { timeline.height(at: $0) } tallest: { timeline.tallest(from: $0) }
	}

	/// The same of one slot, opened and closed before it is open.
	func testTheTallestSlotHoldsThePeakOfAnOpeningOvertakenByAClosing() {
		var timeline = Timeline()
		_ = timeline.settle(stack("a", "b"))
		_ = timeline.apply(stack("a", "b", open: "a"), at: 0, duration: Self.duration)
		_ = timeline.apply(stack("a", "b"), at: 0.05, duration: Self.duration)
		assertTallestHoldsEveryValue(from: 0, to: 0.35) { timeline.frame(of: "a", at: $0)?.height ?? 0 } tallest: { timeline.tallest(of: "a", from: $0) }
	}

	func testSettlingEndsEveryMotionAndTheirCompletions() {
		var timeline = Timeline()
		_ = timeline.settle(stack("d", "a", "b"))
		_ = timeline.apply(stack("a", "d"), at: 0, duration: Self.duration)
		let target = stack("a", "c", open: "a")
		XCTAssertEqual(timeline.settle(target), ["b", "d"])
		XCTAssertNil(timeline.end)
		XCTAssertEqual(timeline.order, ["a", "c"])
		XCTAssertEqual(timeline.frame(of: "a", at: 0.1), Frame(top: 0, height: 200))
		XCTAssertEqual(timeline.frame(of: "c", at: 0.1), Frame(top: 200, height: 60))
		XCTAssertEqual(timeline.height(at: 0.1), 260)
		XCTAssertFalse(timeline.isRaised("a"))
		XCTAssertEqual(timeline.complete(1), Timeline.Completion(dropped: [], lowered: []), "a stale completion does nothing")
	}

	func testACompletionLandsOnce() {
		var timeline = Timeline()
		_ = timeline.settle(stack("a", "b"))
		_ = timeline.apply(stack("a"), at: 0, duration: Self.duration)
		XCTAssertEqual(timeline.complete(1), Timeline.Completion(dropped: ["b"], lowered: []))
		XCTAssertEqual(timeline.complete(1), Timeline.Completion(dropped: [], lowered: []))
		XCTAssertEqual(timeline.order, ["a"])
	}

	// MARK: Playing changes

	/// Plays `changes` from the settled `start`, each over the motion's
	/// length and completed as it ends, sampling every frame to the end.
	/// Checks as each change and completion lands that nothing on screen
	/// moves, and that an inserted slot opens at the bottom of the steady
	/// slot above it.
	private func play(from start: [Slot], _ changes: [Change], file: StaticString = #filePath, line: UInt = #line) -> Playback {
		precondition(zip(changes, changes.dropFirst()).allSatisfy { $0.time < $1.time }, "changes in time order")
		var timeline = Timeline()
		_ = timeline.settle(start)
		// Completions first at a tie: they were scheduled earlier.
		let events: [(time: TimeInterval, rank: Int, event: Event)] = changes.enumerated().flatMap { index, change in
			[
				(time: change.time, rank: 1, event: Event.apply(change.listed)),
				(time: change.time + Self.duration, rank: 0, event: Event.complete(index + 1)),
			]
		}.sorted { ($0.time, $0.rank) < ($1.time, $1.rank) }
		var applied: [Timeline.Change] = []
		var completed: [Timeline.Completion] = []
		var samples: [Sample] = []
		var landed = 0
		let lastFrame = Int(((events.last?.time ?? 0) * Self.frameRate).rounded(.up)) + 1
		for frame in 0 ... lastFrame {
			let time = Double(frame) / Self.frameRate
			while let next = events[safe: landed], next.time <= time {
				let before = sample(timeline, at: next.time, landed: landed)
				switch next.event {
				case let .apply(listed):
					let change = timeline.apply(listed, at: next.time, duration: Self.duration)
					applied.append(change)
					assertLanding(of: change, from: before, on: sample(timeline, at: next.time, landed: landed), file: file, line: line)
				case let .complete(serial):
					let completion = timeline.complete(serial)
					completed.append(completion)
					assertLanding(of: completion, from: before, on: sample(timeline, at: next.time, landed: landed), file: file, line: line)
				}
				landed += 1
			}
			samples.append(sample(timeline, at: time, landed: landed))
		}
		return Playback(timeline: timeline, samples: samples, changes: applied, completions: completed)
	}

	private func sample(_ timeline: Timeline, at time: TimeInterval, landed: Int) -> Sample {
		let frames = timeline.order.compactMap { id in timeline.frame(of: id, at: time).map { (id, $0) } }
		return Sample(
			time: time,
			landed: landed,
			order: timeline.order,
			frames: Dictionary(uniqueKeysWithValues: frames),
			raised: Set(timeline.order.filter { timeline.isRaised($0) }),
			height: timeline.height(at: time),
			tallest: timeline.tallest(from: time),
			topTracks: timeline.slots.mapValues(\.topTracks),
			heightTracks: timeline.slots.mapValues(\.heightTracks),
			stackTracks: timeline.heightTracks,
		)
	}

	// MARK: Checks

	private func assertLanding(of change: Timeline.Change, from before: Sample, on after: Sample, file: StaticString, line: UInt) {
		for (id, frame) in before.frames {
			assertEqual(after.frames[id], frame, "\(id) jumps as change \(change.serial) lands", file: file, line: line)
		}
		XCTAssertEqual(after.height, before.height, accuracy: Self.tolerance, "the stack jumps", file: file, line: line)
		for id in change.inserted {
			let above = after.order.prefix { $0 != id }.last { !change.moved.contains($0) }
			let opening = Frame(top: above.flatMap { after.frames[$0]?.bottom } ?? 0, height: 0)
			assertEqual(after.frames[id], opening, "\(id) opens under the slot above", file: file, line: line)
		}
	}

	private func assertLanding(of completion: Timeline.Completion, from before: Sample, on after: Sample, file: StaticString, line: UInt) {
		for id in completion.dropped {
			XCTAssertEqual(before.frames[id]?.height ?? -1, 0, accuracy: Self.tolerance, "\(id) goes before it closed", file: file, line: line)
		}
		for (id, frame) in after.frames {
			assertEqual(frame, before.frames[id], "\(id) jumps as a change ends", file: file, line: line)
		}
	}

	/// No slot covers another unless it is raised, none shows below no
	/// height, and the stack's height holds them all.
	private func assertNoOverlap(_ playback: Playback, file: StaticString = #filePath, line: UInt = #line) {
		for sample in playback.samples {
			var bottom: CGFloat = 0
			for id in sample.order {
				guard let frame = sample.frames[id] else { return XCTFail("\(id) without a frame", file: file, line: line) }
				let moment = "\(id) at \(sample.time)"
				XCTAssertGreaterThanOrEqual(frame.top, -Self.tolerance, moment, file: file, line: line)
				XCTAssertGreaterThanOrEqual(frame.height, -Self.tolerance, moment, file: file, line: line)
				XCTAssertLessThanOrEqual(frame.bottom, sample.height + Self.tolerance, moment, file: file, line: line)
				guard !sample.raised.contains(id) else { continue }
				XCTAssertGreaterThanOrEqual(frame.top, bottom - Self.tolerance, "\(moment) covers the slot above", file: file, line: line)
				bottom = frame.bottom
			}
		}
	}

	/// Without movers each slot touches the next, top to bottom, at every
	/// frame.
	private func assertGapless(_ playback: Playback, file: StaticString = #filePath, line: UInt = #line) {
		assertNoOverlap(playback, file: file, line: line)
		for sample in playback.samples {
			var bottom: CGFloat = 0
			for id in sample.order {
				XCTAssertEqual(sample.frames[id]?.top ?? -1, bottom, accuracy: Self.tolerance, "a gap above \(id) at \(sample.time)", file: file, line: line)
				bottom = sample.frames[id]?.bottom ?? bottom
			}
			XCTAssertEqual(bottom, sample.height, accuracy: Self.tolerance, "the stack's height at \(sample.time)", file: file, line: line)
		}
	}

	/// From one frame to the next no value moves further than its motions
	/// can at the curve's steepest.
	private func assertNoJumps(_ playback: Playback, file: StaticString = #filePath, line: UInt = #line) {
		for (previous, next) in zip(playback.samples, playback.samples.dropFirst()) {
			let interval = next.time - previous.time
			for (id, frame) in next.frames {
				// One inserted since was checked as it landed.
				guard let was = previous.frames[id] else { continue }
				let topReach = reach(previous.topTracks[id], next.topTracks[id], over: interval)
				let heightReach = reach(previous.heightTracks[id], next.heightTracks[id], over: interval)
				XCTAssertLessThanOrEqual(abs(frame.top - was.top), topReach + Self.tolerance, "\(id)'s top jumps at \(next.time)", file: file, line: line)
				XCTAssertLessThanOrEqual(abs(frame.height - was.height), heightReach + Self.tolerance, "\(id)'s height jumps at \(next.time)", file: file, line: line)
			}
			let stackReach = reach(previous.stackTracks, next.stackTracks, over: interval)
			XCTAssertLessThanOrEqual(abs(next.height - previous.height), stackReach + Self.tolerance, "the stack jumps at \(next.time)", file: file, line: line)
		}
	}

	private func assertSettled(_ playback: Playback, on listed: [Slot], file: StaticString = #filePath, line: UInt = #line) {
		let timeline = playback.timeline
		let end = playback.samples.last?.time ?? 0
		XCTAssertEqual(timeline.order, listed.map(\.id), file: file, line: line)
		XCTAssertTrue(timeline.order.allSatisfy { !timeline.isRaised($0) }, file: file, line: line)
		var top: CGFloat = 0
		for slot in listed {
			XCTAssertEqual(timeline.frame(of: slot.id, at: end), Frame(top: top, height: slot.height), slot.id, file: file, line: line)
			top += slot.height
		}
		XCTAssertEqual(timeline.height(at: end), top, file: file, line: line)
	}

	/// At every frame from `start`, `tallest` is at least every `value`
	/// still to come through `end`, sampled every 10 µs, and at most half a
	/// point over the most of them.
	private func assertTallestHoldsEveryValue(
		from start: TimeInterval,
		to end: TimeInterval,
		file: StaticString = #filePath,
		line: UInt = #line,
		_ value: (TimeInterval) -> CGFloat,
		tallest: (TimeInterval) -> CGFloat,
	) {
		let step = 1e-5
		let values = (0 ... Int(((end - start) / step).rounded(.up))).map { value(min(start + Double($0) * step, end)) }
		// The most to come from each sample on.
		let mostToCome = values.reversed().reduce(into: [CGFloat]()) { most, value in most.append(max(value, most.last ?? value)) }.reversed()
		let peakFrames = (0 ... Int(((end - start) * Self.frameRate).rounded(.up)))
		for frame in peakFrames {
			let time = start + Double(frame) / Self.frameRate
			let most = max(value(time), mostToCome.dropFirst(Int(((time - start) / step).rounded(.up))).first ?? 0)
			let held = tallest(time)
			XCTAssertGreaterThanOrEqual(held, most - Self.tolerance, "the tallest misses a value to come at \(time)", file: file, line: line)
			XCTAssertLessThanOrEqual(held, most + 0.5, "the tallest is too tall at \(time)", file: file, line: line)
		}
		XCTAssertGreaterThan(values.max() ?? 0, max(values.first ?? 0, values.last ?? 0) + 5, "the motion peaks between its ends", file: file, line: line)
	}

	private func assertEqual(_ frame: Frame?, _ expected: Frame?, _ message: String, file: StaticString, line: UInt) {
		guard let frame, let expected else { return XCTAssertEqual(frame, expected, message, file: file, line: line) }
		XCTAssertEqual(frame.top, expected.top, accuracy: Self.tolerance, message, file: file, line: line)
		XCTAssertEqual(frame.height, expected.height, accuracy: Self.tolerance, message, file: file, line: line)
	}

	// MARK: Helpers

	/// The furthest a value can move in `interval` under the tracks it had
	/// at either end of it.
	private func reach(_ earlier: [MotionTrack]?, _ later: [MotionTrack]?, over interval: TimeInterval) -> CGFloat {
		let tracks = ((earlier ?? []) + (later ?? [])).reduce(into: [MotionTrack]()) { tracks, track in
			if !tracks.contains(track) { tracks.append(track) }
		}
		let speed = CGFloat(Self.steepestSlope * interval / Self.duration)
		return tracks.reduce(0) { $0 + abs($1.delta) } * speed
	}

	/// Rows, `open` at the open card's height.
	private func stack(_ ids: String..., open: String? = nil) -> [Slot] {
		ids.map { Slot(id: $0, height: $0 == open ? Self.openHeight : Self.rowHeight) }
	}
}

private extension Array {
	subscript(safe index: Int) -> Element? {
		indices.contains(index) ? self[index] : nil
	}
}
