import AppKit
import CoreGraphics
import Foundation

/// What the Mac reports at one moment.
public struct PresenceReading: Equatable, Sendable {
	/// Seconds since the last keyboard, mouse or trackpad input.
	public var idleSeconds: TimeInterval
	/// The screen is locked (or the session is switched away).
	public var isScreenLocked: Bool
	/// The frontmost app's bundle id.
	public var frontmostBundleId: String?

	public init(idleSeconds: TimeInterval, isScreenLocked: Bool, frontmostBundleId: String?) {
		self.idleSeconds = idleSeconds
		self.isScreenLocked = isScreenLocked
		self.frontmostBundleId = frontmostBundleId
	}
}

/// The platform readings the monitor polls; real ones by default, fakes in
/// the tests.
public protocol PresenceSensors: Sendable {
	/// Seconds since the last input event of any kind.
	func idleSeconds() -> TimeInterval
	/// The frontmost app's bundle id.
	@MainActor func frontmostBundleId() -> String?
}

/// `CGEventSource` for idle time and `NSWorkspace` for the frontmost app;
/// neither needs an entitlement.
public struct SystemPresenceSensors: PresenceSensors {
	public init() {}

	public func idleSeconds() -> TimeInterval {
		// `kCGAnyInputEventType`: the time since input of any kind.
		guard let anyInput = CGEventType(rawValue: ~0) else { return 0 }
		return CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: anyInput)
	}

	@MainActor
	public func frontmostBundleId() -> String? {
		NSWorkspace.shared.frontmostApplication?.bundleIdentifier
	}
}

/// Senses whether you are at the Mac (present, away or locked) and which
/// app is frontmost, and pushes each change into the core as a
/// `PresenceUpdate` (architecture §4: platform sensing stays on the platform
/// side). Only changes are sent.
///
/// Sources: the frontmost app from `NSWorkspace` activation notifications,
/// screen lock from the `com.apple.screenIsLocked` and
/// `com.apple.screenIsUnlocked` distributed notifications (undocumented but
/// long-standing) plus session switches, and idle time polled from
/// `CGEventSource`.
@MainActor
public final class PresenceMonitor {
	/// The default away threshold, `awayAfter` (architecture §6.2).
	public static let defaultAwayAfter: TimeInterval = 5 * 60
	/// How often idle time is read.
	public static let idlePollInterval: TimeInterval = 15

	/// The last update sent.
	public private(set) var lastSent: PresenceUpdate?
	/// The away threshold.
	public var awayAfter: TimeInterval {
		didSet { evaluate() }
	}

	private let sensors: any PresenceSensors
	private let send: @MainActor (PresenceUpdate) -> Void
	private var isScreenLocked = false
	private var observers: [(NotificationCenter, NSObjectProtocol)] = []
	private var idleTimer: Timer?

	/// - Parameters:
	///   - send: receives each changed update on the main actor; it calls the
	///     hub with `await` in a task of its own.
	public init(
		sensors: any PresenceSensors = SystemPresenceSensors(),
		awayAfter: TimeInterval = PresenceMonitor.defaultAwayAfter,
		send: @escaping @MainActor (PresenceUpdate) -> Void,
	) {
		self.sensors = sensors
		self.awayAfter = awayAfter
		self.send = send
	}

	/// The presence a reading means: locked beats away, away beats present.
	public nonisolated static func update(for reading: PresenceReading, awayAfter: TimeInterval) -> PresenceUpdate {
		let state: PresenceState =
			if reading.isScreenLocked {
				.locked
			} else if reading.idleSeconds >= awayAfter {
				.away
			} else {
				.present
			}
		return PresenceUpdate(state: state, frontmostBundleId: reading.frontmostBundleId)
	}

	/// Starts observing and sends the first update.
	public func start() {
		guard observers.isEmpty else { return }
		let workspace = NSWorkspace.shared.notificationCenter
		observe(workspace, NSWorkspace.didActivateApplicationNotification) { $0.evaluate() }
		observe(workspace, NSWorkspace.sessionDidResignActiveNotification) { $0.screenLockChanged(true) }
		observe(workspace, NSWorkspace.sessionDidBecomeActiveNotification) { $0.screenLockChanged(false) }
		let distributed = DistributedNotificationCenter.default()
		observe(distributed, Notification.Name("com.apple.screenIsLocked")) { $0.screenLockChanged(true) }
		observe(distributed, Notification.Name("com.apple.screenIsUnlocked")) { $0.screenLockChanged(false) }
		let timer = Timer(timeInterval: Self.idlePollInterval, repeats: true) { [weak self] _ in
			MainActor.assumeIsolated { self?.evaluate() }
		}
		timer.tolerance = Self.idlePollInterval / 3
		RunLoop.main.add(timer, forMode: .common)
		idleTimer = timer
		evaluate()
	}

	/// Stops observing.
	public func stop() {
		for (center, observer) in observers {
			center.removeObserver(observer)
		}
		observers.removeAll()
		idleTimer?.invalidate()
		idleTimer = nil
	}

	/// The screen locked or unlocked.
	public func screenLockChanged(_ isLocked: Bool) {
		isScreenLocked = isLocked
		evaluate()
	}

	/// Reads the sensors and sends the update if it changed.
	public func evaluate() {
		let reading = PresenceReading(
			idleSeconds: sensors.idleSeconds(),
			isScreenLocked: isScreenLocked,
			frontmostBundleId: sensors.frontmostBundleId(),
		)
		let update = Self.update(for: reading, awayAfter: awayAfter)
		guard update != lastSent else { return }
		lastSent = update
		send(update)
	}

	private func observe(
		_ center: NotificationCenter,
		_ name: Notification.Name,
		_ action: @escaping @MainActor (PresenceMonitor) -> Void,
	) {
		let observer = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
			MainActor.assumeIsolated {
				guard let self else { return }
				action(self)
			}
		}
		observers.append((center, observer))
	}
}
