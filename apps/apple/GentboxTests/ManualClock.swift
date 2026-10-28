import Foundation

/// A hand-wound motion clock: completions run as a test winds it past
/// their time, so a trace samples every frame of a motion deterministically.
@MainActor
final class ManualClock: MotionClock {
	private(set) var now: TimeInterval
	private var pending: [(time: TimeInterval, work: @MainActor () -> Void)] = []

	init(at time: TimeInterval) {
		now = time
	}

	func whenEnded(at time: TimeInterval, _ work: @escaping @MainActor () -> Void) {
		pending.append((time, work))
	}

	/// Runs every completion due by `time`, earliest first, those scheduled
	/// first first at a tie.
	func advance(to time: TimeInterval) {
		while let next = pending.indices.min(by: { pending[$0].time < pending[$1].time }), pending[next].time <= time {
			let due = pending.remove(at: next)
			now = due.time
			due.work()
		}
		now = time
	}
}
