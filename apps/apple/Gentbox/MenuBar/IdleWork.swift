import Foundation

/// Work that can wait for a quiet moment: one piece per main run-loop turn,
/// a display frame apart, so no frame waits on more than one piece.
///
/// On macOS 26 every native button and bezelled text field draws through a
/// SwiftUI graph of its own, and making one and drawing it the first time
/// costs a few milliseconds. The inbox builds them here, ahead of the click
/// that shows them, so the click's frame only reveals what already exists.
@MainActor
enum IdleWork {
	private static var queue: [() -> Void] = []
	private static var isScheduled = false
	/// One frame at 120 Hz.
	private static let spacing: TimeInterval = 1.0 / 120

	static func enqueue(_ work: @escaping () -> Void) {
		queue.append(work)
		scheduleNext()
	}

	private static func scheduleNext() {
		guard !isScheduled, !queue.isEmpty else { return }
		isScheduled = true
		DispatchQueue.main.asyncAfter(deadline: .now() + spacing) {
			MainActor.assumeIsolated {
				isScheduled = false
				if !queue.isEmpty {
					queue.removeFirst()()
				}
				scheduleNext()
			}
		}
	}
}
