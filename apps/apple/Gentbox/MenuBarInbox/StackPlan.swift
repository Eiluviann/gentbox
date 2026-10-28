import CoreGraphics

/// One change of a stack of slots, diffed by the slots' stable ids (ADR
/// 0013, steps 1 and 2): the card's parts, the list's rows, a row's parts.
/// Every slot gets a frame before and a frame after the change, in one
/// merged order, so the change is one motion from the one stack to the
/// other, each slot touching the next and none covering another but a
/// mover's.
///
/// - Slots in both stacks keep their relative order, but for the fewest
///   that must cross the others: the movers, which slide above them.
/// - A removed slot ends at no height where it was, after the slot it
///   followed: the nearest slot before it that does not move.
/// - An inserted slot starts at no height at the bottom of the nearest
///   slot above it that does not move, where it was before the change.
struct StackPlan<ID: Hashable> {
	struct Slot: Equatable {
		let id: ID
		let height: CGFloat
	}

	struct Frame: Equatable {
		var top: CGFloat
		var height: CGFloat

		var bottom: CGFloat {
			top + height
		}
	}

	/// Every slot of either stack, top to bottom: the after stack's order,
	/// the removed slots among it. Also the drawing order, movers aside.
	let order: [ID]
	let before: [ID: Frame]
	let after: [ID: Frame]
	let inserted: Set<ID>
	/// In the before stack and not after, slots still leaving included.
	let removed: Set<ID>
	let movers: Set<ID>

	/// - Parameters:
	///   - before: the stack the previous change aimed at, slots still
	///     leaving included at no height.
	///   - after: the stack to show now.
	init(before: [Slot], after: [Slot]) {
		precondition(Set(before.map(\.id)).count == before.count, "a slot listed twice before the change")
		precondition(Set(after.map(\.id)).count == after.count, "a slot listed twice after the change")
		let beforeIndex = Dictionary(uniqueKeysWithValues: before.enumerated().map { ($0.element.id, $0.offset) })
		let afterHeight = Dictionary(uniqueKeysWithValues: after.map { ($0.id, $0.height) })
		let survivors = after.filter { beforeIndex[$0.id] != nil }
		let movers = Self.movers(among: survivors, beforeIndex: beforeIndex)
		let order = Self.merge(before: before.map(\.id), after: after.map(\.id), movers: movers)
		self.order = order
		self.movers = movers
		inserted = Set(after.map(\.id)).subtracting(beforeIndex.keys)
		removed = Set(before.map(\.id)).subtracting(afterHeight.keys)
		self.after = Self.stacked(order) { afterHeight[$0] ?? 0 }
		self.before = Self.beforeFrames(order: order, before: before, inserted: inserted, movers: movers)
	}

	/// The survivors that must cross the others: all but the longest run
	/// that keeps its order, the run that keeps the most height among
	/// equally long ones, so a small row slides past an open card rather
	/// than the card past it.
	private static func movers(among survivors: [Slot], beforeIndex: [ID: Int]) -> Set<ID> {
		let indices = survivors.map { beforeIndex[$0.id] ?? 0 }
		var runs: [(length: Int, height: CGFloat, previous: Int?)] = []
		for (end, index) in indices.enumerated() {
			var best: (length: Int, height: CGFloat, previous: Int?) = (1, survivors[end].height, nil)
			for start in 0 ..< end where indices[start] < index {
				let run = runs[start]
				let candidate = (length: run.length + 1, height: run.height + survivors[end].height, previous: Optional(start))
				if candidate.length > best.length || (candidate.length == best.length && candidate.height > best.height) {
					best = candidate
				}
			}
			runs.append(best)
		}
		var kept: Set<ID> = []
		var cursor = runs.indices.max { left, right in
			(runs[left].length, runs[left].height) < (runs[right].length, runs[right].height)
		}
		while let end = cursor {
			kept.insert(survivors[end].id)
			cursor = runs[end].previous
		}
		return Set(survivors.map(\.id)).subtracting(kept)
	}

	/// The after stack's order with each removed slot after the slot it
	/// followed before the change: the nearest one placed that does not
	/// move (first, if none), so it never rides across the stack with a
	/// mover, and leaving slots in a run keep their order.
	private static func merge(before: [ID], after: [ID], movers: Set<ID>) -> [ID] {
		var order = after
		let listed = Set(after)
		for (index, id) in before.enumerated() where !listed.contains(id) {
			let predecessor = before[..<index].last { order.contains($0) && !movers.contains($0) }
			let place = predecessor.flatMap { order.firstIndex(of: $0) }.map { $0 + 1 } ?? 0
			order.insert(id, at: place)
		}
		return order
	}

	/// Where each slot was: its place in the before stack, or for an
	/// inserted one no height at the bottom of the nearest steady slot above
	/// it (the stack's top, if none).
	private static func beforeFrames(order: [ID], before: [Slot], inserted: Set<ID>, movers: Set<ID>) -> [ID: Frame] {
		var frames = stacked(before.map(\.id)) { id in before.first { $0.id == id }?.height ?? 0 }
		var steadyBottom: CGFloat = 0
		for id in order {
			if inserted.contains(id) {
				frames[id] = Frame(top: steadyBottom, height: 0)
			}
			if !movers.contains(id), let frame = frames[id] {
				steadyBottom = frame.bottom
			}
		}
		return frames
	}

	private static func stacked(_ order: [ID], height: (ID) -> CGFloat) -> [ID: Frame] {
		var frames: [ID: Frame] = [:]
		var top: CGFloat = 0
		for id in order {
			let frame = Frame(top: top, height: height(id))
			frames[id] = frame
			top = frame.bottom
		}
		return frames
	}
}
