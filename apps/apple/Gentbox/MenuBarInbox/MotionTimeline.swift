import CoreGraphics
import QuartzCore

/// A cubic Bézier timing curve from (0, 0) to (1, 1), as Core Animation
/// draws one: the motion model works out what the render server shows at
/// any moment from the same curve.
struct MotionCurve: Equatable {
	let x1: Double
	let y1: Double
	let x2: Double
	let y2: Double

	/// Eased at both ends, the landing drawn out.
	static let standard = MotionCurve(x1: 0.2, y1: 0, x2: 0, y2: 1)

	var timingFunction: CAMediaTimingFunction {
		CAMediaTimingFunction(controlPoints: Float(x1), Float(y1), Float(x2), Float(y2))
	}

	/// How far along the motion is at `progress` of its time, both 0…1.
	func value(at progress: Double) -> Double {
		guard progress > 0 else { return 0 }
		guard progress < 1 else { return 1 }
		return Self.bezier(parameter(at: progress), y1, y2)
	}

	/// The curve's parameter whose x is `progress`. x rises with it while x1
	/// and x2 lie in 0…1: Newton's method lands in a few steps where x is
	/// steep, and halving finds it where Newton strays.
	private func parameter(at progress: Double) -> Double {
		var parameter = progress
		for _ in 0 ..< 8 {
			let miss = Self.bezier(parameter, x1, x2) - progress
			if abs(miss) < 1e-12 { return parameter }
			let slope = Self.slope(parameter, x1, x2)
			guard slope > 1e-6 else { break }
			parameter -= miss / slope
			guard (0 ... 1).contains(parameter) else { break }
		}
		var low = 0.0
		var high = 1.0
		for _ in 0 ..< 40 {
			let middle = (low + high) / 2
			if Self.bezier(middle, x1, x2) < progress {
				low = middle
			} else {
				high = middle
			}
		}
		return (low + high) / 2
	}

	private static func bezier(_ parameter: Double, _ first: Double, _ second: Double) -> Double {
		let rest = 1 - parameter
		return 3 * rest * rest * parameter * first + 3 * rest * parameter * parameter * second + parameter * parameter * parameter
	}

	private static func slope(_ parameter: Double, _ first: Double, _ second: Double) -> Double {
		let rest = 1 - parameter
		return 3 * rest * rest * first + 6 * rest * parameter * (second - first) + 3 * parameter * parameter * (1 - second)
	}
}

/// One change's motion of one value, as an additive animation: the value
/// shows `delta` away from its target as the change starts and reaches it
/// at the end of the curve. A value's motions add up, so a change landing
/// mid-motion adds its own and leaves the others running: nothing jumps and
/// nothing restarts.
struct MotionTrack: Equatable {
	/// The change that started it.
	let change: Int
	let start: TimeInterval
	let duration: TimeInterval
	let delta: CGFloat

	var end: TimeInterval {
		start + duration
	}

	/// How far from its target the value shows at `time`.
	func offset(at time: TimeInterval, curve: MotionCurve = .standard) -> CGFloat {
		delta * CGFloat(1 - curve.value(at: (time - start) / duration))
	}
}

/// The motion of one stack of slots through every change still running
/// (ADR 0013, steps 2 and 4), as values: each slot's target frame and the
/// tracks that move it there, which `InboxTransition` commits to Core
/// Animation as they are. What any slot shows at any moment is its target
/// plus its tracks, so the tests check the motion frame by frame.
///
/// Every slot carries every running change's tracks, and all of a change's
/// tracks share one clock: with both stacks of each change gapless, the sum
/// stays gapless. A slot inserted mid-motion takes the running tracks of
/// the steady slot above it, as if it had sat there at no height all along.
struct MotionTimeline<ID: Hashable> {
	typealias Slot = StackPlan<ID>.Slot
	typealias Frame = StackPlan<ID>.Frame

	struct SlotMotion: Equatable {
		var top: CGFloat
		var height: CGFloat
		var topTracks: [MotionTrack]
		var heightTracks: [MotionTrack]
		/// The change whose completion ends the slot, while it leaves.
		var leavingChange: Int?
		/// The change whose completion drops it back among the others, while
		/// it slides above the slots it passes.
		var raisingChange: Int?

		func frame(at time: TimeInterval, curve: MotionCurve) -> Frame {
			Frame(
				top: top + topTracks.reduce(0) { $0 + $1.offset(at: time, curve: curve) },
				height: height + heightTracks.reduce(0) { $0 + $1.offset(at: time, curve: curve) },
			)
		}
	}

	/// What one change does to the stack, for the views to commit.
	struct Change {
		let serial: Int
		let plan: StackPlan<ID>
		/// Shown now and not before, a slot whose removal had ended included.
		let inserted: Set<ID>
		/// Starting to leave now.
		let removed: Set<ID>
		/// Taken back while still leaving.
		let returned: Set<ID>
		let moved: Set<ID>
		let resized: Set<ID>
		/// The tracks to add per slot: this change's, and for an inserted
		/// slot the running ones it takes from above.
		let topTracks: [ID: [MotionTrack]]
		let heightTracks: [ID: [MotionTrack]]
		/// The stack's own height, which its parent slot moves by.
		let heightTrack: MotionTrack?
	}

	/// What a change's completion ends.
	struct Completion: Equatable {
		/// Slots that finished leaving, out of the stack now.
		let dropped: [ID]
		/// Movers back among the others.
		let lowered: [ID]
	}

	/// A span of time and the most a value can show in it.
	/// What a value's tracks add at one moment: the falling ones, which show
	/// it above its target, and the rising ones, which show it below.
	private struct Offsets {
		let falling: CGFloat
		let rising: CGFloat

		var sum: CGFloat {
			falling + rising
		}
	}

	/// A span of a motion and the most it can show there: the falling
	/// tracks as at its start, the rising ones as at its end.
	private struct BoundedSpan {
		let start: TimeInterval
		let end: TimeInterval
		let fallingAtStart: CGFloat
		let risingAtEnd: CGFloat

		var bound: CGFloat {
			fallingAtStart + risingAtEnd
		}

		init(from start: (time: TimeInterval, offsets: Offsets), to end: (time: TimeInterval, offsets: Offsets)) {
			self.start = start.time
			self.end = end.time
			fallingAtStart = start.offsets.falling
			risingAtEnd = end.offsets.rising
		}
	}

	let curve: MotionCurve
	/// Top to bottom, slots still leaving included: the drawing order,
	/// raised slots aside.
	private(set) var order: [ID] = []
	private(set) var slots: [ID: SlotMotion] = [:]
	/// The stack's height's own tracks.
	private(set) var heightTracks: [MotionTrack] = []
	private(set) var lastChange = 0

	init(curve: MotionCurve = .standard) {
		self.curve = curve
	}

	/// The stack's height once every motion has ended.
	var targetHeight: CGFloat {
		order.reduce(0) { $0 + (slots[$1]?.height ?? 0) }
	}

	/// When the last running track ends.
	var end: TimeInterval? {
		let ends = slots.values.flatMap { $0.topTracks + $0.heightTracks }.map(\.end) + heightTracks.map(\.end)
		return ends.max()
	}

	func frame(of id: ID, at time: TimeInterval) -> Frame? {
		slots[id]?.frame(at: time, curve: curve)
	}

	func height(at time: TimeInterval) -> CGFloat {
		targetHeight + heightTracks.reduce(0) { $0 + $1.offset(at: time, curve: curve) }
	}

	func isRaised(_ id: ID) -> Bool {
		slots[id]?.raisingChange != nil
	}

	/// The tallest the stack shows from `time` to the end of its motion, half
	/// a point at most over: a window or scroll view sized to it once
	/// holds every frame of it.
	func tallest(from time: TimeInterval) -> CGFloat {
		Self.largest(of: targetHeight, movedBy: heightTracks, from: time, curve: curve)
	}

	/// The tallest slot `id` shows from `time` to the end of its motion, half
	/// a point at most over.
	func tallest(of id: ID, from time: TimeInterval) -> CGFloat {
		guard let slot = slots[id] else { return 0 }
		return Self.largest(of: slot.height, movedBy: slot.heightTracks, from: time, curve: curve)
	}

	/// Moves the stack to `listed` from where it shows at `time`, over
	/// `duration`.
	mutating func apply(_ listed: [Slot], at time: TimeInterval, duration: TimeInterval) -> Change {
		precondition(duration > 0, "a motion takes time; settle for none")
		dropTracks(endedBy: time)
		lastChange += 1
		let serial = lastChange
		let plan = StackPlan(before: order.map { Slot(id: $0, height: slots[$0]?.height ?? 0) }, after: listed)
		let listedIds = Set(listed.map(\.id))
		var next: [ID: SlotMotion] = [:]
		var inserted: Set<ID> = []
		var removed: Set<ID> = []
		var returned: Set<ID> = []
		var resized: Set<ID> = []
		var addedTop: [ID: [MotionTrack]] = [:]
		var addedHeight: [ID: [MotionTrack]] = [:]
		var steadyAbove: ID?
		for id in plan.order {
			guard let from = plan.before[id], let to = plan.after[id] else { preconditionFailure("a slot without frames") }
			var slot = slots[id] ?? Self.arriving(under: steadyAbove.flatMap { next[$0] }, before: serial)
			if slots[id] == nil {
				inserted.insert(id)
				addedTop[id] = slot.topTracks
			}
			if from.top != to.top {
				let track = MotionTrack(change: serial, start: time, duration: duration, delta: from.top - to.top)
				slot.topTracks.append(track)
				addedTop[id, default: []].append(track)
			}
			if from.height != to.height {
				let track = MotionTrack(change: serial, start: time, duration: duration, delta: from.height - to.height)
				slot.heightTracks.append(track)
				addedHeight[id, default: []].append(track)
				if listedIds.contains(id), !plan.inserted.contains(id), slot.leavingChange == nil {
					resized.insert(id)
				}
			}
			slot.top = to.top
			slot.height = to.height
			if !listedIds.contains(id), slot.leavingChange == nil {
				removed.insert(id)
				slot.leavingChange = serial
			}
			if listedIds.contains(id), slot.leavingChange != nil {
				returned.insert(id)
				slot.leavingChange = nil
			}
			if plan.movers.contains(id) {
				slot.raisingChange = serial
			} else {
				steadyAbove = id
			}
			next[id] = slot
		}
		order = plan.order
		slots = next
		let heightDelta = plan.before.values.reduce(0) { $0 + $1.height } - plan.after.values.reduce(0) { $0 + $1.height }
		let heightTrack = heightDelta == 0 ? nil : MotionTrack(change: serial, start: time, duration: duration, delta: heightDelta)
		heightTracks += [heightTrack].compactMap { $0 }
		return Change(
			serial: serial, plan: plan, inserted: inserted, removed: removed, returned: returned, moved: plan.movers,
			resized: resized, topTracks: addedTop, heightTracks: addedHeight, heightTrack: heightTrack,
		)
	}

	/// Ends change `serial`: its leaving slots go and its movers drop back,
	/// unless a later change took them back or moved them again. Running it
	/// twice does nothing more.
	mutating func complete(_ serial: Int) -> Completion {
		let dropped = order.filter { slots[$0]?.leavingChange == serial }
		let lowered = order.filter { slots[$0]?.raisingChange == serial && !dropped.contains($0) }
		for id in dropped {
			slots[id] = nil
		}
		for id in lowered {
			slots[id]?.raisingChange = nil
		}
		order.removeAll { dropped.contains($0) }
		return Completion(dropped: dropped, lowered: lowered)
	}

	/// Shows `listed` at once, every motion ended: a change without motion,
	/// or views leaving the window. The slots it drops.
	mutating func settle(_ listed: [Slot]) -> [ID] {
		let listedIds = Set(listed.map(\.id))
		let dropped = order.filter { !listedIds.contains($0) }
		var top: CGFloat = 0
		var next: [ID: SlotMotion] = [:]
		for slot in listed {
			next[slot.id] = SlotMotion(
				top: top, height: slot.height, topTracks: [], heightTracks: [], leavingChange: nil, raisingChange: nil,
			)
			top += slot.height
		}
		order = listed.map(\.id)
		slots = next
		heightTracks = []
		return dropped
	}

	/// A slot inserted mid-motion: at no height under the steady slot above
	/// it through every change still running, so it moves with it, raised
	/// as long as that slot is.
	private static func arriving(under above: SlotMotion?, before serial: Int) -> SlotMotion {
		let running = ((above?.topTracks ?? []) + (above?.heightTracks ?? [])).filter { $0.change < serial }
		return SlotMotion(
			top: 0, height: 0, topTracks: running, heightTracks: [], leavingChange: nil,
			raisingChange: above?.raisingChange,
		)
	}

	/// The most `target` moved by `tracks` shows from `time` on: never less,
	/// at most `tallestSlack` more. Each track's offset only falls toward 0 or
	/// only rises, so over any span the falling ones show most at its start
	/// and the rising ones at its end: the span whose bound is highest is
	/// halved until no bound tops the most found by more than the slack, each
	/// halving one sum of the tracks at the middle. A motion that only grows
	/// or only shrinks takes two sums; one that peaks between, a few dozen.
	/// The change's commit waits on it, so it must be quick: sampling every
	/// millisecond took milliseconds.
	private static func largest(
		of target: CGFloat,
		movedBy tracks: [MotionTrack],
		from time: TimeInterval,
		curve: MotionCurve,
	) -> CGFloat {
		func offsets(at moment: TimeInterval) -> (time: TimeInterval, offsets: Offsets) {
			let split = tracks.reduce(into: (falling: CGFloat(0), rising: CGFloat(0))) { split, track in
				let offset = track.offset(at: moment, curve: curve)
				if track.delta > 0 { split.falling += offset } else { split.rising += offset }
			}
			return (moment, Offsets(falling: split.falling, rising: split.rising))
		}
		let first = offsets(at: time)
		guard let end = tracks.map(\.end).max(), end > time else { return target + first.offsets.sum }
		let last = offsets(at: end)
		var largest = max(first.offsets.sum, last.offsets.sum)
		var spans = [BoundedSpan(from: first, to: last)]
		// Motions that nearly cancel keep every bound high: the budget ends
		// the halving there, a few points over at most.
		for _ in 0 ..< halvingBudget {
			guard let highest = spans.indices.max(by: { spans[$0].bound < spans[$1].bound }),
			      spans[highest].bound > largest + tallestSlack
			else { break }
			let halved = spans.remove(at: highest)
			let middle = offsets(at: (halved.start + halved.end) / 2)
			largest = max(largest, middle.offsets.sum)
			let halves = [
				BoundedSpan(from: (halved.start, Offsets(falling: halved.fallingAtStart, rising: 0)), to: middle),
				BoundedSpan(from: middle, to: (halved.end, Offsets(falling: 0, rising: halved.risingAtEnd))),
			]
			spans += halves.filter { $0.bound > largest }
		}
		return target + max(largest, spans.map(\.bound).max() ?? largest)
	}

	/// How far over the tallest may be. Past what it draws, the card's window
	/// is clear and the list's scroll view clipped, so half a point never
	/// shows; a tenth took twice the halvings.
	private static var tallestSlack: CGFloat {
		0.5
	}

	private static var halvingBudget: Int {
		128
	}

	private mutating func dropTracks(endedBy time: TimeInterval) {
		for id in order {
			slots[id]?.topTracks.removeAll { $0.end <= time }
			slots[id]?.heightTracks.removeAll { $0.end <= time }
		}
		heightTracks.removeAll { $0.end <= time }
	}
}
