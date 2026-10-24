import AppKit
import GentboxCore
import os

/// Starts the hub, the status item and the presence monitor at launch, and
/// stops the hub at quit so the store closes and `API/port` goes away.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
	private let app = AppModel()
	private var statusItem: StatusItemController?

	func applicationWillFinishLaunching(_: Notification) {
		// Gentbox has no credential fields. AppKit's autofill heuristics
		// otherwise take the reply field for a one-time-code field: each time
		// it takes focus they walk every key view in the panel and build
		// Safari's code-autofill window (~6 ms of a 8.3 ms frame). Registered
		// defaults live in this process only.
		UserDefaults.standard.register(defaults: [
			"NSAutoFillHeuristicsEnabled": false,
			"NSAutoFillRequiresTextContentTypeForOneTimeCodeOnMac": true,
		])
	}

	func applicationDidFinishLaunching(_: Notification) {
		statusItem = StatusItemController(app: app)
		app.start()
		app.loginItem.registerOnFirstLaunch()
	}

	func applicationWillTerminate(_: Notification) {
		app.stop()
	}
}
