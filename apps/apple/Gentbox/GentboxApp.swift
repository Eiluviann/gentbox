import SwiftUI

/// Gentbox: an inbox for your AI agents, in the menu bar. The app is the hub
/// (architecture §3): it runs the core in-process and serves the local API
/// the agents post to.
@main
struct GentboxApp: App {
	@NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

	var body: some Scene {
		// A menu-bar app (LSUIElement) with no window of its own yet; the
		// main window with History and Settings comes with PR 5.
		Settings { EmptyView() }
	}
}
