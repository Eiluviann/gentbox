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
/// card start on one clock, and ends it once, when its motion has run. The
/// last completion lets the window and the scroll geometry shrink to the
/// target, which they outgrew to hold every frame of the motion.
@MainActor
final class InboxTransition {
	private struct WeakStack {
		weak var stack: (any MotionStack)?
	}

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
	/// completion lands once, `duration` later.
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
		clock.whenEnded(at: time + duration) { [weak self] in self?.complete(change) }
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
