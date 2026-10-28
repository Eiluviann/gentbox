import QuartzCore

/// The motion's clock: Core Animation's in the app, a hand-wound one in the
/// tests.
@MainActor
protocol MotionClock: AnyObject {
	var now: TimeInterval { get }
	/// Runs `work` once the render server has drawn every frame up to `time`,
	/// or the motion was dropped.
	func whenEnded(at time: TimeInterval, _ work: @escaping @MainActor () -> Void)
}

/// One change of the window's card, moved in the transaction that moves the
/// content: the card's height once the change has run, the tallest it shows
/// on the way, and the change's clock.
struct CardMotion: Equatable {
	let height: CGFloat
	let tallest: CGFloat
	let change: Int
	let start: TimeInterval
	let duration: TimeInterval
}

/// One change of the coordinator, as a view's parts take it: the number its
/// stacks end it by, its clock, and whether zooms give way to fades.
struct MotionChange: Equatable {
	let change: Int
	let time: TimeInterval
	let duration: TimeInterval
	let reducesMotion: Bool
}

/// How the content's card changes, for the window around it.
enum CardChange: Equatable {
	/// At once, every motion dropped.
	case settled(height: CGFloat)
	case moving(CardMotion)
	/// Every motion has run: the window may shrink to the card.
	case motionEnded
}

extension CALayer {
	/// Adds `track` as an additive animation of `keyPath` under `key`: the
	/// value shows `delta` away from its model as the track starts and
	/// reaches it on the motion's curve, held at `delta` until then.
	@MainActor
	func addMotion(_ track: MotionTrack, keyPath: String, key: String) {
		let animation = CABasicAnimation(keyPath: keyPath)
		animation.isAdditive = true
		animation.fromValue = track.delta
		animation.toValue = 0
		animation.beginTime = convertTime(track.start, from: nil)
		animation.duration = track.duration
		animation.fillMode = .backwards
		animation.timingFunction = Tokens.motionCurve
		add(animation, forKey: key)
	}

	/// Each change's tracks of `keyPath`, summed into one additive animation
	/// keyed by the change: a slot inserted mid-motion can take two of one
	/// change from the slot above it, its top's and its height's.
	@MainActor
	func addMotions(_ tracks: [MotionTrack], keyPath: String) {
		for (change, tracks) in Dictionary(grouping: tracks, by: \.change) {
			guard let first = tracks.first else { continue }
			let summed = MotionTrack(change: change, start: first.start, duration: first.duration, delta: tracks.reduce(0) { $0 + $1.delta })
			addMotion(summed, keyPath: keyPath, key: "motion.\(keyPath).\(change)")
		}
	}

	/// Keeps `keyPath` drawn `track.delta` from its model while the track
	/// runs: what a change settles as it ends, before its motion has run
	/// (`InboxTransition`), shows once the motion has.
	@MainActor
	func holdMotion(_ track: MotionTrack, keyPath: String, key: String) {
		let animation = CABasicAnimation(keyPath: keyPath)
		animation.isAdditive = true
		animation.fromValue = track.delta
		animation.toValue = track.delta
		animation.beginTime = convertTime(track.start, from: nil)
		animation.duration = track.duration
		add(animation, forKey: key)
	}

	/// Drops every animation whose key starts with `prefix`.
	func removeMotions(keyedFrom prefix: String) {
		for key in animationKeys() ?? [] where key.hasPrefix(prefix) {
			removeAnimation(forKey: key)
		}
	}
}

/// Core Animation's clock: a completion rides an animation of its own, which
/// changes nothing it is drawn on and ends at the motion's end, so it lands
/// on the render server's frame rather than on a timer's.
@MainActor
final class LayerMotionClock: MotionClock {
	private let layer = CALayer()
	private var sentinels = 0

	/// - Parameter host: a layer drawn while the motion runs; the clock adds
	///   a sublayer of no size to it.
	init(on host: CALayer?) {
		host?.addSublayer(layer)
	}

	var now: TimeInterval {
		CACurrentMediaTime()
	}

	func whenEnded(at time: TimeInterval, _ work: @escaping @MainActor () -> Void) {
		sentinels += 1
		let start = now
		CATransaction.begin()
		CATransaction.setCompletionBlock {
			MainActor.assumeIsolated { work() }
		}
		let sentinel = CABasicAnimation(keyPath: "opacity")
		sentinel.isAdditive = true
		sentinel.fromValue = 0
		sentinel.toValue = 0
		sentinel.beginTime = layer.convertTime(start, from: nil)
		sentinel.duration = max(time - start, 0.001)
		layer.add(sentinel, forKey: "motion.clock.\(sentinels)")
		CATransaction.commit()
	}
}

/// The inbox's one coordinator of change (ADR 0013): it numbers each
/// change, commits it in one transaction, so every stack and the window's
/// card start on one clock, and ends it once, two frames before its motion
/// has run. The last completion lets the window and the scroll geometry
/// shrink to the target, which they outgrew to hold every frame of the
/// motion.
@MainActor
final class InboxTransition {
	private struct WeakStack {
		weak var stack: (any MotionStack)?
	}

	/// Two frames at 120 Hz, one at 60. The work of a change's end commits
	/// while its motion still draws: by the motion's last frame the display
	/// has slowed, and a commit then comes late, a hitch. The motion has
	/// under a third of a point left to move then, which the window's room
	/// below the card holds, and its tracks run out with nothing committed.
	private static let endLead: TimeInterval = 2.0 / 120

	private let clock: any MotionClock
	private var stacks: [WeakStack] = []
	private var lastChange = 0
	private(set) var running: Set<Int> = []
	/// Every running change has ended.
	var onLastCompletion: (() -> Void)?

	init(clock: any MotionClock) {
		self.clock = clock
	}

	var now: TimeInterval {
		clock.now
	}

	var isMoving: Bool {
		!running.isEmpty
	}

	/// Ends `stack`'s part of every change with the change; kept weakly, so a
	/// row's own stack goes with the row.
	func add(_ stack: any MotionStack) {
		stacks.removeAll { $0.stack == nil || $0.stack === stack }
		stacks.append(WeakStack(stack: stack))
	}

	/// Runs one change: `commit` sets every target and adds the change's
	/// motion, as change `change` starting at `time`, in one transaction. Its
	/// completion lands once, two frames short of `duration` later (half of
	/// a change shorter than four).
	@discardableResult
	func run(for duration: TimeInterval, _ commit: (_ change: Int, _ time: TimeInterval) -> Void) -> Int {
		precondition(duration > 0, "a change without motion settles")
		lastChange += 1
		let change = lastChange
		let time = clock.now
		CATransaction.begin()
		CATransaction.setDisableActions(true)
		running.insert(change)
		commit(change, time)
		clock.whenEnded(at: time + duration - min(Self.endLead, duration / 2)) { [weak self] in self?.complete(change) }
		CATransaction.commit()
		return change
	}

	/// Ends change `change` in every stack; once more does nothing.
	func complete(_ change: Int) {
		guard running.remove(change) != nil else { return }
		for stack in liveStacks {
			stack.complete(change)
		}
		if running.isEmpty {
			onLastCompletion?()
		}
	}

	/// Ends every change at once, each stack at its target: their
	/// completions, still to land, do nothing.
	func settle() {
		running.removeAll()
		CATransaction.begin()
		CATransaction.setDisableActions(true)
		for stack in liveStacks {
			stack.settle()
		}
		CATransaction.commit()
	}

	private var liveStacks: [any MotionStack] {
		stacks.compactMap(\.stack)
	}
}
