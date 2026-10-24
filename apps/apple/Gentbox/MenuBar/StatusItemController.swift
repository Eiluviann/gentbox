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
	/// The same content laid out at its ideal height, off screen: what the
	/// panel's height should be.
	private let measuringHost: NSHostingController<MenuBarInbox>
	private let panel: InboxPanel
	/// The expanded item as last seen, and when it last changed: the panel's
	/// height waits for the spring to settle.
	private var lastExpanded: InboxModel.Expanded?
	private var lastExpansionChange = Date.distantPast
	private var fitting: Task<Void, Never>?
	private var outsideClicks: Any?
	#if DEBUG
	private var trace: AnimationTrace?
	#endif

	init(app: AppModel) {
		self.app = app
		host = NSHostingController(rootView: MenuBarInbox(app: app))
		// The panel is sized by hand from the content's ideal size: letting
		// the content size the window feeds its frame back into SwiftUI's
		// layout, which AppKit aborts as a loop.
		host.sizingOptions = []
		measuringHost = NSHostingController(rootView: MenuBarInbox(app: app, isMeasuring: true))
		measuringHost.sizingOptions = []
		panel = InboxPanel(content: host.view)
		super.init()
		panel.onClose = { [weak self] in self?.closePanel() }
		configureButton()
		updateButton()
		fitOnChange()
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
			#if DEBUG
			if let path = ProcessInfo.processInfo.environment["GENTBOX_TRACE_DIR"], !path.isEmpty {
				let measuringHost = measuringHost
				trace = AnimationTrace(
					directory: URL(fileURLWithPath: path), panel: panel,
					measure: { measuringHost.sizeThatFits(in: NSSize(width: Tokens.popoverWidth, height: 2000)).height },
					inbox: { [weak app] in app?.inbox },
				)
				trace?.run()
			}
			#endif
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
		app.inbox?.presentationChanged(isPresented: true)
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
		guard panel.isVisible else { return }
		panel.orderOut(nil)
		if let outsideClicks {
			NSEvent.removeMonitor(outsideClicks)
		}
		outsideClicks = nil
		// The next opening starts again from the next-up item, unfiltered.
		app.inbox?.presentationChanged(isPresented: false)
	}

	/// Sizes the panel to its content and hangs it under the status item, at
	/// once (opening) or eased (while open).
	private func placePanel(animated: Bool = false) {
		// `NSScreen.main` is the key window's screen, nil while the app runs in
		// the background with no key window.
		guard let screen = item.button?.window?.screen ?? NSScreen.main ?? NSScreen.screens.first else { return }
		let anchor = item.button?.window?.frame ?? .zero
		let ideal = measuringHost.sizeThatFits(in: NSSize(width: Tokens.popoverWidth, height: screen.visibleFrame.height))
		let size = NSSize(width: Tokens.popoverWidth, height: ceil(ideal.height))
		let target = InboxPanel.frame(size: size, under: anchor, in: screen.visibleFrame)
		guard target != panel.frame else { return }
		if animated {
			NSAnimationContext.runAnimationGroup { context in
				context.duration = 0.2
				context.timingFunction = CAMediaTimingFunction(name: .easeOut)
				panel.animator().setFrame(target, display: false)
			}
		} else {
			panel.setFrame(target, display: false)
		}
	}

	/// The panel fits its content, but never during an expansion: while the
	/// card springs open or shut, the panel holds its height (the content
	/// hangs from the top and the list scrolls inside), and only once the
	/// spring has settled does the panel ease to the new height. A panel that
	/// jumped to the end size while the content was mid-spring is what made
	/// the accordion glitch.
	private func fitOnChange() {
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
				// An inbox that starts while the panel shows is shown too.
				if self.panel.isVisible {
					self.app.inbox?.presentationChanged(isPresented: true)
				}
				self.scheduleFit()
				self.fitOnChange()
			}
		}
	}

	/// Fits the panel once the last expansion has settled.
	private func scheduleFit() {
		let expanded = app.inbox?.expanded
		if expanded != lastExpanded {
			lastExpanded = expanded
			lastExpansionChange = Date()
		}
		fitting?.cancel()
		fitting = Task { @MainActor in
			let settle = Self.settle - Date().timeIntervalSince(lastExpansionChange)
			if settle > 0 {
				guard (try? await Task.sleep(for: .seconds(settle))) != nil else { return }
			}
			if panel.isVisible {
				placePanel(animated: true)
			}
		}
	}

	/// The spring's settling time (`.spring` is 0.5 s) plus a margin.
	static let settle: TimeInterval = 0.55

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
