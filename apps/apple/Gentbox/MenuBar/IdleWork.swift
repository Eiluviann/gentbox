import AppKit

/// Work that can wait for a quiet moment: one piece per main run-loop turn,
/// a display frame apart, so no frame waits on more than one piece.
///
/// On macOS 26 every native button and bezelled text field draws through a
/// SwiftUI graph of its own, and making one and drawing it the first time
/// costs a few milliseconds. The inbox builds them here, ahead of the click
/// that shows them, so the click's frame only reveals what already exists.
///
/// A piece runs only when nothing waits: no input queued and no motion in
/// flight. Otherwise it is tried again a frame later.
@MainActor
enum IdleWork {
	private static var queue: [() -> Void] = []
	private static var isScheduled = false
	/// One frame at 120 Hz.
	private static let spacing: TimeInterval = 1.0 / 120
	/// Until when a motion runs: no piece starts before.
	private static var heldUntil: TimeInterval = 0
	/// Input that a piece would make wait.
	private static let input: NSEvent.EventTypeMask = [
		.leftMouseDown, .leftMouseUp, .rightMouseDown, .rightMouseUp, .keyDown, .scrollWheel, .leftMouseDragged,
	]

	/// No piece runs for `duration`: a motion has started.
	static func holdOff(for duration: TimeInterval) {
		heldUntil = max(heldUntil, ProcessInfo.processInfo.systemUptime + duration)
	}

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
				if !queue.isEmpty, isQuiet {
					queue.removeFirst()()
				}
				scheduleNext()
			}
		}
	}

	private static var isQuiet: Bool {
		ProcessInfo.processInfo.systemUptime >= heldUntil
			&& NSApp.nextEvent(matching: input, until: .distantPast, inMode: .default, dequeue: false) == nil
	}
}
