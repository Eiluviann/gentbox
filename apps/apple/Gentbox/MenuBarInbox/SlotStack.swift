import AppKit
import QuartzCore

/// A stack the inbox's one coordinator moves: it ends a change when the
/// change's motion has run, and drops every motion at once.
@MainActor
protocol MotionStack: AnyObject {
	func complete(_ change: Int)
	/// Shows the stack's target at once, every motion ended.
	func settle()
}

/// The views of one stack of slots, moved by one `MotionTimeline` (ADR
/// 0013, step 4). A change sets every slot's frame to its target at once,
/// so layout and clicks see the target, and adds the change's tracks as
/// additive animations, one per change and property, keyed by the change so
/// no key is used twice: the render server runs them on the one curve and
/// clock, and no frame of the motion touches the main thread. Content fades
/// and zooms by `MotionSpec`. Slots are drawn in the stack's order, movers
/// above the rest until their change ends.
@MainActor
final class SlotStack<ID: Hashable>: MotionStack {
	struct Entry {
		let id: ID
		let view: SlotView
		let height: CGFloat
	}

	private static var raisedZ: CGFloat {
		1000
	}

	private(set) var timeline = MotionTimeline<ID>()
	/// Every slot shown or still leaving.
	private(set) var views: [ID: SlotView] = [:]
	var width: CGFloat
	/// A slot that finished leaving, hidden and its motion dropped, for its
	/// owner to keep or discard.
	var onDrop: ((ID, SlotView) -> Void)?
	private let container: NSView
	/// The timeline's serial for each of the coordinator's changes this
	/// stack took part in: a row's parts change less often than the list.
	private var serials: [Int: Int] = [:]

	init(in container: NSView, width: CGFloat) {
		self.container = container
		self.width = width
	}

	/// The stack's height once every motion has ended.
	var targetHeight: CGFloat {
		timeline.targetHeight
	}

	/// Moves the stack to `entries` from where it shows at `time`, as the
	/// coordinator's change `change`.
	@discardableResult
	func apply(_ entries: [Entry], change: Int, at time: TimeInterval, duration: TimeInterval, reducesMotion: Bool) -> MotionTimeline<ID>.Change {
		adopt(entries)
		let applied = timeline.apply(entries.map(Self.slot), at: time, duration: duration)
		serials[change] = applied.serial
		for id in applied.plan.order {
			guard let view = views[id], let slot = timeline.slots[id] else { continue }
			if applied.inserted.contains(id) {
				view.endMotion()
				view.isHidden = false
			}
			view.frame = NSRect(x: 0, y: slot.top, width: width, height: slot.height)
			Self.add(applied.topTracks[id] ?? [], to: view.layer, keyPath: "position.y")
			Self.add(applied.heightTracks[id] ?? [], to: view.layer, keyPath: "bounds.size.height")
		}
		moveContent(of: applied, at: time, reducesMotion: reducesMotion)
		layerInOrder()
		return applied
	}

	func complete(_ change: Int) {
		guard let serial = serials.removeValue(forKey: change) else { return }
		let completion = timeline.complete(serial)
		for id in completion.dropped {
			drop(id)
		}
		layerInOrder()
	}

	/// Shows `entries` at once, every motion ended: a change without motion.
	func settle(_ entries: [Entry]) {
		adopt(entries)
		serials.removeAll()
		for id in timeline.settle(entries.map(Self.slot)) {
			drop(id)
		}
		for entry in entries {
			guard let slot = timeline.slots[entry.id] else { continue }
			entry.view.endMotion()
			entry.view.isHidden = false
			entry.view.isLeaving = false
			entry.view.content.alphaValue = 1
			entry.view.frame = NSRect(x: 0, y: slot.top, width: width, height: slot.height)
		}
		layerInOrder()
	}

	func settle() {
		let listed = timeline.order.compactMap { id -> Entry? in
			guard let view = views[id], let slot = timeline.slots[id], slot.leavingChange == nil else { return nil }
			return Entry(id: id, view: view, height: slot.height)
		}
		settle(listed)
	}

	private func adopt(_ entries: [Entry]) {
		for entry in entries {
			precondition(views[entry.id].map { $0 === entry.view } ?? true, "one slot, one view")
			views[entry.id] = entry.view
			if entry.view.superview !== container {
				container.addSubview(entry.view)
			}
		}
	}

	private func moveContent(of applied: MotionTimeline<ID>.Change, at time: TimeInterval, reducesMotion: Bool) {
		for id in applied.inserted {
			views[id]?.isLeaving = false
			views[id]?.moveContent(MotionSpec.content(.insert, reducesMotion: reducesMotion), at: time)
		}
		for id in applied.removed {
			guard let view = views[id] else { continue }
			view.isLeaving = true
			view.moveContent(Self.fromShown(view, at: time, to: MotionSpec.content(.remove, reducesMotion: reducesMotion)), at: time)
		}
		for id in applied.returned {
			guard let view = views[id] else { continue }
			view.isLeaving = false
			view.moveContent(Self.fromShown(view, at: time, to: MotionSpec.content(.insert, reducesMotion: reducesMotion)), at: time)
		}
	}

	private func drop(_ id: ID) {
		guard let view = views.removeValue(forKey: id) else { return }
		view.endMotion()
		view.isHidden = true
		onDrop?(id, view)
	}

	/// The stack's order is the drawing order, raised slots above it.
	private func layerInOrder() {
		for (index, id) in timeline.order.enumerated() {
			views[id]?.layer?.zPosition = CGFloat(index) + (timeline.isRaised(id) ? Self.raisedZ : 0)
		}
	}

	/// Each change's tracks on one property, summed into one additive
	/// animation: a slot inserted mid-motion can take two of one change from
	/// the slot above it, its top's and its height's.
	private static func add(_ tracks: [MotionTrack], to layer: CALayer?, keyPath: String) {
		guard let layer else { return }
		for (change, tracks) in Dictionary(grouping: tracks, by: \.change) {
			guard let first = tracks.first else { continue }
			let animation = CABasicAnimation(keyPath: keyPath)
			animation.isAdditive = true
			animation.fromValue = tracks.reduce(0) { $0 + $1.delta }
			animation.toValue = 0
			animation.beginTime = layer.convertTime(first.start, from: nil)
			animation.duration = first.duration
			animation.fillMode = .backwards
			animation.timingFunction = Tokens.motionCurve
			layer.add(animation, forKey: "motion.\(keyPath).\(change)")
		}
	}

	/// `spec`, starting from what the slot's content shows: a slot taken
	/// back as it leaves, or leaving as it arrives.
	private static func fromShown(_ view: SlotView, at time: TimeInterval, to spec: MotionSpec.Content) -> MotionSpec.Content {
		let shown = view.shownContent(at: time)
		return MotionSpec.Content(
			fromOpacity: shown.opacity, toOpacity: spec.toOpacity, fromScale: shown.scale, toScale: spec.toScale,
			duration: spec.duration, delay: 0,
		)
	}

	private static func slot(_ entry: Entry) -> MotionTimeline<ID>.Slot {
		MotionTimeline<ID>.Slot(id: entry.id, height: entry.height)
	}
}
