import AppKit
import GentboxCore
import os

/// Starts the hub, the status item and the presence monitor at launch, and
/// stops the hub at quit so the store closes and `API/port` goes away.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
	private let app = AppModel()
	private var statusItem: StatusItemController?

	func applicationDidFinishLaunching(_: Notification) {
		statusItem = StatusItemController(app: app)
		app.start()
		app.loginItem.registerOnFirstLaunch()
	}

	func applicationWillTerminate(_: Notification) {
		app.stop()
	}
}
