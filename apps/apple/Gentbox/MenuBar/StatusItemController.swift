import AppKit
import GentboxCore
import Observation
import SwiftUI

/// The menu-bar item and its inbox: the Gentbox mark, the count of agents
/// waiting on you, and the inbox in a non-activating panel under the item
/// (`InboxPanel`), closed by a click elsewhere, Escape or the item again.
@MainActor
final class StatusItemController: NSObject {
	/// What the UI tests look the item up by.
	static let accessibilityIdentifier = "gentbox.status-item"
	/// A launch argument that opens the inbox at launch, for the UI tests
	/// (with "hide the menu bar automatically" on, XCUITest's synthetic
	/// pointer never reveals the status item) and for checking the panel
	/// against a window manager without clicking.
	static let openAtLaunchKey = "GentboxOpenInboxAtLaunch"

	private let app: AppModel
	private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
	private let host: NSHostingController<MenuBarInbox>
	private let panel: InboxPanel
	private var outsideClicks: Any?

	init(app: AppModel) {
		self.app = app
		host = NSHostingController(rootView: MenuBarInbox(app: app))
		// The panel is sized by hand from the content's ideal size: letting
		// the content size the window feeds its frame back into SwiftUI's
		// layout, which AppKit aborts as a loop.
		host.sizingOptions = []
		panel = InboxPanel(content: host.view)
		super.init()
		configureButton()
		updateButton()
		resizeOnChange()
		if UserDefaults.standard.bool(forKey: Self.openAtLaunchKey) {
			if let path = ProcessInfo.processInfo.environment["GENTBOX_SNAPSHOT_PATH"], !path.isEmpty {
				// Snapshots are for comparing with the design boards, which are
				// light; `GENTBOX_SNAPSHOT_APPEARANCE=dark` checks dark mode.
				let isDark = ProcessInfo.processInfo.environment["GENTBOX_SNAPSHOT_APPEARANCE"] == "dark"
				panel.appearance = NSAppearance(named: isDark ? .darkAqua : .aqua)
				snapshotOnChange(to: URL(fileURLWithPath: path))
			}
			// Shown without taking the keyboard: launching it must not steal
			// focus from the app you are in.
			showPanel(takingKeyboard: false)
		}
	}

	private func configureButton() {
		guard let button = item.button else { return }
		button.imagePosition = .imageLeading
		button.font = .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .medium)
		button.setAccessibilityIdentifier(Self.accessibilityIdentifier)
		button.setAccessibilityLabel("Gentbox")
		button.target = self
		button.action = #selector(togglePanel(_:))
	}

	@objc private func togglePanel(_: NSStatusBarButton) {
		if panel.isVisible {
			closePanel()
		} else {
			showPanel(takingKeyboard: true)
		}
	}

	/// Opened from the status item it takes the keyboard (the card's ↩, ⌘⌫
	/// and R) without activating the app.
	private func showPanel(takingKeyboard: Bool) {
		app.inbox?.requestRead()
		placePanel()
		if takingKeyboard {
			panel.makeKeyAndOrderFront(nil)
		} else {
			panel.orderFrontRegardless()
		}
		// A click anywhere else closes it, as a menu would; clicks inside the
		// panel and its menus are this app's own and never reach this monitor.
		outsideClicks = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
			MainActor.assumeIsolated { self?.closePanel() }
		}
	}

	private func closePanel() {
		panel.orderOut(nil)
		if let outsideClicks {
			NSEvent.removeMonitor(outsideClicks)
		}
		outsideClicks = nil
		// The next opening starts again from the next-up item, unfiltered.
		app.inbox?.resetExpansion()
	}

	/// Sizes the panel to its content and hangs it under the status item.
	private func placePanel() {
		// `NSScreen.main` is the key window's screen, nil while the app runs in
		// the background with no key window.
		guard let screen = item.button?.window?.screen ?? NSScreen.main ?? NSScreen.screens.first else { return }
		let anchor = item.button?.window?.frame ?? .zero
		let ideal = host.sizeThatFits(in: NSSize(width: Tokens.popoverWidth, height: screen.visibleFrame.height))
		let size = NSSize(width: Tokens.popoverWidth, height: ceil(ideal.height))
		panel.setFrame(InboxPanel.frame(size: size, under: anchor, in: screen.visibleFrame), display: true)
	}

	/// Keeps the panel fitted to its content as the inbox changes.
	private func resizeOnChange() {
		withObservationTracking {
			_ = app.phase
			_ = app.inbox?.inbox
			_ = app.inbox?.failure
			_ = app.inbox?.expanded
			_ = app.inbox?.filter
			_ = app.inbox?.selection
		} onChange: { [weak self] in
			Task { @MainActor in
				guard let self else { return }
				if self.panel.isVisible {
					self.placePanel()
				}
				self.resizeOnChange()
			}
		}
	}

	/// A test hook: writes the panel's content to a PNG after every inbox
	/// change. The app draws its own window, so no Screen Recording
	/// permission is involved (a runner without it captures only the desktop
	/// picture behind the window).
	private func snapshotOnChange(to url: URL) {
		withObservationTracking {
			_ = app.inbox?.inbox
			_ = app.inbox?.failure
			_ = app.inbox?.expanded
			_ = app.inbox?.filter
			_ = app.inbox?.selection
		} onChange: { [weak self] in
			Task { @MainActor in
				// After the spring animations settle.
				try? await Task.sleep(for: .milliseconds(1200))
				self?.writeSnapshot(to: url)
				self?.snapshotOnChange(to: url)
			}
		}
	}

	private func writeSnapshot(to url: URL) {
		let view = host.view
		guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
		view.cacheDisplay(in: view.bounds, to: bitmap)
		// The cache leaves the panel's own surface out: lay it under.
		let size = NSSize(width: bitmap.pixelsWide, height: bitmap.pixelsHigh)
		guard let opaque = NSBitmapImageRep(
			bitmapDataPlanes: nil, pixelsWide: bitmap.pixelsWide, pixelsHigh: bitmap.pixelsHigh,
			bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
			colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0,
		) else { return }
		// Resolved under the panel's appearance before drawing into the bitmap.
		var surface = NSColor.white
		panel.effectiveAppearance.performAsCurrentDrawingAppearance {
			surface = Tokens.surface.usingColorSpace(.sRGB) ?? .white
		}
		NSGraphicsContext.saveGraphicsState()
		NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: opaque)
		surface.setFill()
		NSRect(origin: .zero, size: size).fill()
		// `draw(in:)` copies, which would wipe the surface: lay it over instead.
		bitmap.draw(
			in: NSRect(origin: .zero, size: size), from: .zero, operation: .sourceOver,
			fraction: 1, respectFlipped: false, hints: nil,
		)
		NSGraphicsContext.restoreGraphicsState()
		try? opaque.representation(using: .png, properties: [:])?.write(to: url, options: .atomic)
	}

	/// Redraws the mark and the count, then waits for the next change.
	private func updateButton() {
		withObservationTracking {
			render()
		} onChange: { [weak self] in
			Task { @MainActor in self?.updateButton() }
		}
	}

	private func render() {
		guard let button = item.button else { return }
		let waiting = app.inbox?.waitingCount ?? 0
		let state: MenuBarMark.State =
			if waiting > 0 {
				.blocked
			} else if app.inbox?.hasUnread == true {
				.unread
			} else {
				.idle
			}
		button.image = MenuBarMark.image(for: state)
		button.title = waiting > 0 ? "\(waiting)" : ""
		button.setAccessibilityValue(Self.accessibilityValue(waiting: waiting, state: state, phase: app.phase))
	}

	private static func accessibilityValue(waiting: Int, state: MenuBarMark.State, phase: AppModel.Phase) -> String {
		if case .failed = phase { return "Not running" }
		switch state {
		case .blocked: return "\(waiting) waiting on you"
		case .unread: return "Unread messages"
		case .idle: return "Nothing open"
		}
	}
}
