import AppKit
import GentboxCore
import Observation

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
	private let content: InboxContentView
	/// The inbox's own panel, used for nothing else: popups, toasts and
	/// Settings get windows of their own.
	private let panel: InboxPanel
	/// Where the open panel hangs: its left edge and its top, fixed from
	/// opening to closing. Only the height changes while it shows.
	private var placement: (x: CGFloat, top: CGFloat, maxHeight: CGFloat)?
	private var outsideClicks: Any?
	#if DEBUG || GENTBOX_HOOKS
	private var trace: AnimationTrace?
	private var demo: DemoDriver?
	#endif

	init(app: AppModel) {
		self.app = app
		content = InboxContentView(app: app)
		#if DEBUG || GENTBOX_HOOKS
		panel = Self.isRecording ? RecordingInboxPanel(content: content) : InboxPanel(content: content)
		#else
		panel = InboxPanel(content: content)
		#endif
		super.init()
		Interactions.panel = panel
		content.onHeightChange = { [weak self] height, animated in self?.fitPanel(height: height, animated: animated) }
		NotificationCenter.default.addObserver(
			self, selector: #selector(screensChanged(_:)),
			name: NSApplication.didChangeScreenParametersNotification, object: nil,
		)
		panel.onClose = { [weak self] in self?.closePanel() }
		configureButton()
		updateButton()
		#if DEBUG || GENTBOX_HOOKS
		if Self.isOffscreen {
			// AppKit saves an item's visibility under its autosave name; under
			// its own name, hiding it here leaves the real item shown on the
			// next launch.
			item.autosaveName = "gentbox.offscreen"
			item.isVisible = false
		}
		#endif
		presentWhenRunning()
		#if DEBUG || GENTBOX_HOOKS
		if let path = ProcessInfo.processInfo.environment["GENTBOX_DEMO_DIR"], !path.isEmpty {
			demo = DemoDriver(
				directory: URL(fileURLWithPath: path), panel: panel,
				open: { [weak self] in self?.showPanel(takingKeyboard: false) },
				close: { [weak self] in self?.closePanel() },
				inbox: { [weak app] in app?.inbox },
			)
			demo?.run()
		}
		DebugPanel.install(
			app: app,
			openInbox: { [weak self] in
				guard let self, !self.panel.isVisible || self.panel.isDespawning else { return }
				self.showPanel(takingKeyboard: false)
			},
			closeInbox: { [weak self] in self?.closePanel() },
		)
		// Test, snapshot and trace hooks: none of them reach a Release build.
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
			if let path = ProcessInfo.processInfo.environment["GENTBOX_TRACE_DIR"], !path.isEmpty {
				trace = AnimationTrace(
					directory: URL(fileURLWithPath: path), panel: panel,
					measure: { [content] in content.naturalHeight },
					inbox: { [weak app] in app?.inbox },
					open: { [weak self] in self?.showPanel(takingKeyboard: false) },
					close: { [weak self] in self?.closePanel() },
				)
				trace?.run()
			}
		}
		#endif
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
		if panel.isVisible, !panel.isDespawning {
			closePanel()
		} else {
			showPanel(takingKeyboard: true)
		}
	}

	/// Opened from the status item it takes the keyboard (the card's ↩, ⌘⌫
	/// and R) without activating the app.
	private func showPanel(takingKeyboard: Bool) {
		Interactions.untilNextFrame("Open")
		app.inbox?.requestRead()
		app.inbox?.presentationChanged(isPresented: true)
		// Re-opened while it fades out: it turns back where it is.
		if !panel.isDespawning {
			placePanel()
			guard placement != nil else { return }
		}
		panel.spawn(reduceMotion: Self.reducesMotion)
		if takingKeyboard {
			panel.makeKeyAndOrderFront(nil)
		} else {
			panel.orderFrontRegardless()
		}
		#if DEBUG || GENTBOX_HOOKS
		// The recording harness: your clicks elsewhere must not cut a clip.
		if demo != nil { return }
		#endif
		// A click anywhere else closes it, as a menu would; clicks inside the
		// panel and its menus are this app's own and never reach this monitor.
		outsideClicks = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
			MainActor.assumeIsolated { self?.closePanel() }
		}
	}

	/// Every way of closing (Escape, a click elsewhere, losing the keyboard,
	/// the status item) fades the panel out first; a display going away
	/// closes it at once.
	private func closePanel(animated: Bool = true) {
		guard panel.isVisible, animated ? !panel.isDespawning : true else { return }
		if let outsideClicks {
			NSEvent.removeMonitor(outsideClicks)
		}
		outsideClicks = nil
		if animated {
			panel.despawn(reduceMotion: Self.reducesMotion) { [weak self] in self?.finishClosing() }
		} else {
			finishClosing()
		}
	}

	private func finishClosing() {
		panel.orderOut(nil)
		placement = nil
		// The next opening starts again from the next-up item, unfiltered.
		app.inbox?.presentationChanged(isPresented: false)
	}

	#if DEBUG || GENTBOX_HOOKS
	/// Set by the demo driver, so one launch records both variants.
	static var reduceMotionOverride: Bool?
	#endif

	/// Whether motion fades in place rather than scaling: the Mac's Reduce
	/// Motion, or a test's override.
	static var reducesMotion: Bool {
		#if DEBUG || GENTBOX_HOOKS
		// Tests and recordings show both variants whatever the Mac is set to.
		if let reduceMotionOverride {
			return reduceMotionOverride
		}
		if let override = ProcessInfo.processInfo.environment["GENTBOX_REDUCE_MOTION"] {
			return override == "1"
		}
		#endif
		return NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
	}

	/// Hangs the panel under the status item at its content's height, before
	/// it shows: the place is set once, never animated, and holds until the
	/// panel closes.
	private func placePanel() {
		// `NSScreen.main` is the key window's screen, nil while the app runs in
		// the background with no key window.
		guard let screen = item.button?.window?.screen ?? NSScreen.main ?? NSScreen.screens.first else { return }
		var anchor = item.button?.window?.frame ?? .zero
		var visibleFrame = screen.visibleFrame
		var screenFrame = screen.frame
		#if DEBUG || GENTBOX_HOOKS
		if Self.isOffscreen {
			(anchor, visibleFrame) = Self.offscreen(visibleFrame)
			screenFrame = visibleFrame
		} else if Self.isTestRun, let main = NSScreen.screens.first {
			// The main screen, the one with the menu bar, whichever screen
			// the status item reports.
			anchor = Self.topRightCorner(of: main.frame)
			visibleFrame = main.visibleFrame
			screenFrame = main.frame
		}
		#endif
		let maxHeight = InboxPanel.maxHeight(under: anchor, in: visibleFrame, screen: screenFrame)
		content.maxHeight = maxHeight
		content.settle()
		let frame = InboxPanel.frame(
			size: NSSize(width: Tokens.popoverWidth, height: devicePixelCeiling(content.naturalHeight)),
			under: anchor, in: visibleFrame, screen: screenFrame,
		)
		placement = (frame.minX, frame.maxY, maxHeight)
		panel.place(card: frame, maxCardHeight: maxHeight)
	}

	/// A display was added, removed or rearranged while the panel shows: its
	/// place may be off screen now, and it never moves, so it closes, as a
	/// menu would.
	@objc private func screensChanged(_: Notification) {
		closePanel(animated: false)
	}

	/// An inbox that starts while the panel shows is shown too.
	private func presentWhenRunning() {
		withObservationTracking {
			_ = app.phase
		} onChange: { [weak self] in
			Task { @MainActor in
				guard let self else { return }
				if self.panel.isVisible {
					self.app.inbox?.presentationChanged(isPresented: true)
				}
				self.presentWhenRunning()
			}
		}
	}

	/// The content's height changed: the card follows in the same Core
	/// Animation transaction, on the same curve, so its bottom edge stays on
	/// the content's. The content has already stopped at the screen's height.
	private func fitPanel(height: CGFloat, animated: Bool) {
		guard panel.isVisible, placement != nil else { return }
		panel.setCardHeight(devicePixelCeiling(height), animated: animated)
	}

	/// Up to the next device pixel, so the card ends where the content does.
	private func devicePixelCeiling(_ height: CGFloat) -> CGFloat {
		let scale = panel.backingScaleFactor
		return ceil(height * scale) / scale
	}

	/// Driven by `DemoDriver` for a recording.
	#if DEBUG || GENTBOX_HOOKS
	/// `GENTBOX_TEST_OFFSCREEN=1`: measurements while the user's screen is in
	/// use. The status item is hidden and the panel hangs as it would under
	/// it, but to the right of every display, so nothing shows.
	static var isOffscreen: Bool {
		ProcessInfo.processInfo.environment["GENTBOX_TEST_OFFSCREEN"] == "1"
	}

	/// A test, demo or measuring run, which keeps its state apart: the panel
	/// hangs in the main screen's top-right corner, under the menu bar,
	/// wherever the status item is, so it covers as little of the screen as
	/// it can while you work.
	static var isTestRun: Bool {
		!(ProcessInfo.processInfo.environment["GENTBOX_STATE_DIR"] ?? "").isEmpty
	}

	/// An anchor at the right end of `screen`'s menu bar.
	static func topRightCorner(of screen: NSRect) -> NSRect {
		let thickness = NSStatusBar.system.thickness
		return NSRect(x: screen.maxX - 1, y: screen.maxY - thickness, width: 1, height: thickness)
	}

	/// A status item's place at the top right of `visibleFrame`, and the
	/// frame, both moved past the right edge of every display.
	private static func offscreen(_ visibleFrame: NSRect) -> (anchor: NSRect, visibleFrame: NSRect) {
		let rightmost = NSScreen.screens.map(\.frame.maxX).max() ?? visibleFrame.maxX
		let shift = rightmost - visibleFrame.minX + 1000
		let anchor = NSRect(x: visibleFrame.maxX - 240, y: visibleFrame.maxY, width: 40, height: 24)
		return (anchor.offsetBy(dx: shift, dy: 0), visibleFrame.offsetBy(dx: shift, dy: 0))
	}
	#endif

	private static var isRecording: Bool {
		#if DEBUG || GENTBOX_HOOKS
		!(ProcessInfo.processInfo.environment["GENTBOX_DEMO_DIR"] ?? "").isEmpty
		#else
		false
		#endif
	}

	#if DEBUG || GENTBOX_HOOKS
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
				// After the motion settles.
				try? await Task.sleep(for: .milliseconds(1200))
				self?.writeSnapshot(to: url)
				self?.snapshotOnChange(to: url)
			}
		}
	}

	private func writeSnapshot(to url: URL) {
		let view: NSView = content
		// The hosting view spans the panel's column; the card is its top.
		let card = NSRect(x: 0, y: 0, width: view.bounds.width, height: panel.cardHeight)
		guard let bitmap = view.bitmapImageRepForCachingDisplay(in: card) else { return }
		view.cacheDisplay(in: card, to: bitmap)
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
	#endif

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
