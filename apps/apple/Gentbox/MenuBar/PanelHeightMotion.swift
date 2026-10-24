import QuartzCore
import SwiftUI

/// The panel's height while its content animates: the same curve and
/// duration as `Tokens.motion`, combined the way SwiftUI combines a change
/// that arrives mid-animation (each change eases its own difference in, on
/// top of the ones still running), so the panel's bottom edge stays with the
/// content's bottom frame by frame.
struct PanelHeightMotion {
	/// The height the content settles at.
	private(set) var target: CGFloat
	private var changes: [(difference: CGFloat, start: CFTimeInterval)] = []
	private let duration: TimeInterval
	private let curve: UnitCurve

	init(height: CGFloat, duration: TimeInterval = Tokens.motionDuration, curve: UnitCurve = Tokens.motionCurve) {
		target = height
		self.duration = duration
		self.curve = curve
	}

	/// Whether a change is still easing in at `time`.
	func isMoving(at time: CFTimeInterval) -> Bool {
		changes.contains { time < $0.start + duration }
	}

	/// The panel's height at `time`.
	func height(at time: CFTimeInterval) -> CGFloat {
		changes.reduce(target) { height, change in
			let progress = min(max((time - change.start) / duration, 0), 1)
			return height - change.difference * (1 - curve.value(at: progress))
		}
	}

	/// The content's height changed to `height` at `time`.
	mutating func retarget(to height: CGFloat, at time: CFTimeInterval) {
		guard height != target else { return }
		changes = changes.filter { time < $0.start + duration }
		changes.append((height - target, time))
		target = height
	}

	/// Jumps to `height`, as on opening.
	mutating func reset(to height: CGFloat) {
		changes = []
		target = height
	}
}
