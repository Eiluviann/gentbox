import AppKit
import QuartzCore
import XCTest

/// A stack inside a slot of another, as a row's parts sit in its slot in
/// the list (ADR 0013, Nesting): both moved by the coordinator's one change
/// on one clock, the inner stack's drawn bottom is the slot's drawn bottom
/// at every frame at 120 Hz, so a part never shows past its row's edge or
/// leaves a gap above the next row, through a burst of inserts, removes and
/// resizes in both stacks.
@MainActor
final class NestedStackTests: XCTestCase {
	private typealias Stack = SlotStack<String>
	private typealias Slot = StackPlan<String>.Slot
	private typealias Frame = StackPlan<String>.Frame

	private struct Change {
		/// After the playback's start.
		let time: TimeInterval
		/// The outer stack's ids, `nested` among them at the parts' height.
		let rows: [String]
		let parts: [Slot]
	}

	private struct Track {
		let delta: Double
		let begin: TimeInterval
		let duration: TimeInterval
	}

	private struct Sample {
		let time: TimeInterval
		/// The nested slot's drawn top and height.
		let slot: Frame
		/// The parts' drawn frames, top to bottom.
		let parts: [Frame]
		let modelPartsHeight: CGFloat

		var partsBottom: CGFloat {
			parts.last?.bottom ?? 0
		}
	}

	private static let frameRate = 120.0
	/// The curve's steepest slope (`MotionTimelineTests`).
	private static let steepestSlope = 4.0451
	private static let tolerance: CGFloat = 1e-6
	private static let width: CGFloat = 380
	private static let rowHeight: CGFloat = 60
	private static let nested = "b"
	/// Far from 0, which Core Animation reads as "now".
	private static let origin: TimeInterval = 1000

	private var duration: TimeInterval {
		Tokens.motionDuration
	}

	private var containers: [NSView] = []

	func testAStackInASlotFillsItExactlyThroughABurst() throws {
		let start = [part("p0", 40), part("p1", 30), part("p2", 50)]
		let changes = [
			Change(time: 0, rows: ["n", "a", "b", "c"], parts: [part("p0", 40), part("q", 24), part("p1", 30), part("p2", 80)]),
			Change(time: 0.05, rows: ["n", "a", "b", "c"], parts: [part("p0", 40), part("q", 24), part("p2", 80)]),
			Change(time: 0.12, rows: ["n", "b", "c"], parts: [part("p0", 20), part("q", 24), part("p2", 80), part("r", 36)]),
		]
		let samples = play(rows: ["a", "b", "c"], parts: start, changes)
		for sample in samples {
			XCTAssertEqual(sample.partsBottom, sample.slot.height, accuracy: Self.tolerance, "the parts fill the slot at \(sample.time)")
			XCTAssertEqual(sample.partsBottom, sample.modelPartsHeight, accuracy: Self.tolerance, "drawn as modelled at \(sample.time)")
			XCTAssertEqual(sample.parts.first?.top ?? 0, 0, accuracy: Self.tolerance, "from the slot's top at \(sample.time)")
			for (upper, lower) in zip(sample.parts, sample.parts.dropFirst()) {
				XCTAssertEqual(lower.top, upper.bottom, accuracy: Self.tolerance, "no gap or overlap at \(sample.time)")
			}
		}
		// Every change's resize of the parts at once: a jump of any one
		// shows above it.
		let reach: CGFloat = 54 + 30 + 16
		let maxStep = reach * Self.steepestSlope / (Self.frameRate * duration) + Self.tolerance
		for (earlier, later) in zip(samples, samples.dropFirst()) {
			XCTAssertLessThanOrEqual(abs(later.slot.height - earlier.slot.height), maxStep, "the slot jumps at \(later.time)")
			XCTAssertLessThanOrEqual(abs(later.slot.top - earlier.slot.top), 2 * Self.rowHeight * Self.steepestSlope / (Self.frameRate * duration))
		}
		let last = try XCTUnwrap(samples.last)
		XCTAssertEqual(last.slot.top, Self.rowHeight, accuracy: 1e-4)
		XCTAssertEqual(last.slot.height, 160, accuracy: 1e-4)
		XCTAssertEqual(last.parts.count, 4, "the removed part dropped")
		let listedHeights: [CGFloat] = [20, 24, 80, 36]
		for (drawn, listed) in zip(last.parts, listedHeights) {
			XCTAssertEqual(drawn.height, listed, accuracy: 1e-4)
		}
	}

	// MARK: Helpers

	/// Plays `changes` on both stacks through one coordinator, the slot's
	/// content sized to hold the tallest its parts get, and samples every
	/// frame from the first change through the last's motion.
	private func play(rows start: [String], parts startParts: [Slot], _ changes: [Change]) -> [Sample] {
		let clock = ManualClock(at: Self.origin)
		let transition = InboxTransition(clock: clock)
		let container = FlippedView(frame: NSRect(x: 0, y: 0, width: Self.width, height: 1000))
		containers.append(container)
		let rows = Stack(in: container, width: Self.width)
		var rowViews: [String: SlotView] = [:]
		let nestedSlot = SlotView()
		rowViews[Self.nested] = nestedSlot
		let parts = Stack(in: nestedSlot.content, width: Self.width)
		var partViews: [String: SlotView] = [:]
		transition.add(rows)
		transition.add(parts)
		parts.settle(entries(startParts, views: &partViews))
		rows.settle(entries(rowSlots(start, partsHeight: parts.targetHeight), views: &rowViews))
		fitContent(of: nestedSlot, to: parts, from: Self.origin)
		var pending = changes[...]
		let end = (changes.last?.time ?? 0) + duration
		return (0 ... Int((end * Self.frameRate).rounded(.up))).map { frame in
			let time = Self.origin + Double(frame) / Self.frameRate
			while let next = pending.first, Self.origin + next.time <= time {
				clock.advance(to: Self.origin + next.time)
				let partEntries = entries(next.parts, views: &partViews)
				let partsHeight = next.parts.reduce(0) { $0 + $1.height }
				let rowEntries = entries(rowSlots(next.rows, partsHeight: partsHeight), views: &rowViews)
				transition.run(for: duration) { change, at in
					parts.apply(partEntries, change: change, at: at, duration: duration, reducesMotion: false)
					rows.apply(rowEntries, change: change, at: at, duration: duration, reducesMotion: false)
					fitContent(of: nestedSlot, to: parts, from: at)
				}
				pending = pending.dropFirst()
			}
			clock.advance(to: time)
			return sample(nestedSlot, parts, at: time)
		}
	}

	private func sample(_ nestedSlot: SlotView, _ parts: Stack, at time: TimeInterval) -> Sample {
		let partFrames = parts.timeline.order.compactMap { id in parts.views[id].map { drawnFrame(of: $0, at: time) } }
		return Sample(
			time: time, slot: drawnFrame(of: nestedSlot, at: time), parts: partFrames, modelPartsHeight: parts.timeline.height(at: time),
		)
	}

	private func drawnFrame(of view: NSView, at time: TimeInterval) -> Frame {
		Frame(
			top: view.frame.minY + offset(tracks(on: view.layer, keyPath: "position.y"), at: time),
			height: view.frame.height + offset(tracks(on: view.layer, keyPath: "bounds.size.height"), at: time),
		)
	}

	/// The slot's owner lays its content out at the tallest its parts get,
	/// for the slot to clip.
	private func fitContent(of slot: SlotView, to parts: Stack, from time: TimeInterval) {
		let height = max(parts.timeline.tallest(from: time), parts.targetHeight)
		slot.content.frame = NSRect(x: 0, y: 0, width: Self.width, height: height)
	}

	private func tracks(on layer: CALayer?, keyPath: String) -> [Track] {
		guard let layer else { return [] }
		return (layer.animationKeys() ?? []).filter { $0.hasPrefix("motion.\(keyPath).") }.compactMap { key in
			guard let animation = layer.animation(forKey: key) as? CABasicAnimation, animation.isAdditive,
			      let delta = (animation.fromValue as? NSNumber)?.doubleValue
			else { return nil }
			return Track(delta: delta, begin: animation.beginTime, duration: animation.duration)
		}
	}

	private func offset(_ tracks: [Track], at time: TimeInterval) -> CGFloat {
		tracks.reduce(0) { sum, track in
			let progress = (time - track.begin) / track.duration
			guard progress < 1 else { return sum }
			return sum + CGFloat(track.delta * (1 - MotionCurve.standard.value(at: progress)))
		}
	}

	private func entries(_ listed: [Slot], views: inout [String: SlotView]) -> [Stack.Entry] {
		var made: [Stack.Entry] = []
		for slot in listed {
			let view = views[slot.id] ?? SlotView()
			views[slot.id] = view
			made.append(Stack.Entry(id: slot.id, view: view, height: slot.height))
		}
		return made
	}

	private func rowSlots(_ ids: [String], partsHeight: CGFloat) -> [Slot] {
		ids.map { Slot(id: $0, height: $0 == Self.nested ? partsHeight : Self.rowHeight) }
	}

	private func part(_ id: String, _ height: CGFloat) -> Slot {
		Slot(id: id, height: height)
	}
}
