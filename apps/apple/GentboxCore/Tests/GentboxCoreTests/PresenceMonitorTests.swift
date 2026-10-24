import Foundation
import Testing
@testable import GentboxCore

/// Fake sensors the test moves by hand.
final class FakeSensors: PresenceSensors, @unchecked Sendable {
	var idle: TimeInterval = 0
	var frontmost: String? = "com.mitchellh.ghostty"

	func idleSeconds() -> TimeInterval { idle }
	func frontmostBundleId() -> String? { frontmost }
}

@Suite("Presence mapping")
struct PresenceMappingTests {
	@Test(arguments: [
		(0.0, false, PresenceState.present),
		(299.0, false, .present),
		(300.0, false, .away),
		(3600.0, false, .away),
		(0.0, true, .locked),
		(3600.0, true, .locked),
	])
	func readingMapsToState(idle: TimeInterval, isLocked: Bool, expected: PresenceState) {
		let reading = PresenceReading(idleSeconds: idle, isScreenLocked: isLocked, frontmostBundleId: "com.apple.Terminal")
		let update = PresenceMonitor.update(for: reading, awayAfter: 300)
		#expect(update.state == expected)
		#expect(update.frontmostBundleId == "com.apple.Terminal")
	}
}

@Suite("Presence monitor")
@MainActor
struct PresenceMonitorTests {
	@Test func sendsOnlyChanges() {
		let sensors = FakeSensors()
		var sent: [PresenceUpdate] = []
		let monitor = PresenceMonitor(sensors: sensors, awayAfter: 300) { sent.append($0) }

		monitor.evaluate()
		monitor.evaluate()
		#expect(sent.map(\.state) == [.present])

		sensors.frontmost = "com.anthropic.claudefordesktop"
		monitor.evaluate()
		#expect(sent.last?.frontmostBundleId == "com.anthropic.claudefordesktop")

		sensors.idle = 301
		monitor.evaluate()
		monitor.screenLockChanged(true)
		monitor.screenLockChanged(false)
		#expect(sent.map(\.state) == [.present, .present, .away, .locked, .away])
	}

	@Test func aShorterThresholdTakesEffectAtOnce() {
		let sensors = FakeSensors()
		sensors.idle = 120
		var sent: [PresenceUpdate] = []
		let monitor = PresenceMonitor(sensors: sensors, awayAfter: 300) { sent.append($0) }
		monitor.evaluate()
		monitor.awayAfter = 60
		#expect(sent.map(\.state) == [.present, .away])
	}
}
