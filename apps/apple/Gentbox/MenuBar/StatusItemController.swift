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
	private let layout = PanelLayout()
	private let host: NSHostingController<MenuBarInbox>
	/// The same content laid out at its natural height, off screen: the
	/// panel's height on opening, and whether content that scrolls fits again.
	/// While the panel shows, the content reports its own height.
	private let measuringHost: NSHostingController<MenuBarInbox>
	/// The inbox's own panel, used for nothing else: popups, toasts and
	/// Settings get windows of their own.
	private let panel: InboxPanel
	/// Where the open panel hangs: its left edge and its top, fixed from
	/// opening to closing. Only the height changes while it shows.
	private var placement: (x: CGFloat, top: CGFloat, maxHeight: CGFloat)?
	private var height = PanelHeightMotion(height: 0)
	/// A new height, waiting for the next display frame to start from.
	private var pendingHeight: CGFloat?
	private var heightLink: CADisplayLink?
	private var outsideClicks: Any?
	#if DEBUG
	private var trace: AnimationTrace?
	#endif

	init(app: AppModel) {
		self.app = app
		host = NSHostingController(rootView: MenuBarInbox(app: app, layout: layout))
		// The panel is sized by hand from the content's ideal size: letting
		// the content size the window feeds its frame back into SwiftUI's
		// layout, which AppKit aborts as a loop.
		host.sizingOptions = []
		measuringHost = NSHostingController(rootView: MenuBarInbox(app: app, layout: layout, isMeasuring: true))
		measuringHost.sizingOptions = []
		panel = InboxPanel(content: host.view)
		super.init()
		layout.onNaturalHeightChange = { [weak self] natural in self?.fitPanel(natural: natural) }
		NotificationCenter.default.addObserver(
			self, selector: #selector(screensChanged(_:)),
			name: NSApplication.didChangeScreenParametersNotification, object: nil,
		)
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
				trace = AnimationTrace(
					directory: URL(fileURLWithPath: path), panel: panel,
					measure: { [layout] in layout.naturalHeight },
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
		guard placement != nil else { return }
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
		heightLink?.invalidate()
		heightLink = nil
		pendingHeight = nil
		placement = nil
		if let outsideClicks {
			NSEvent.removeMonitor(outsideClicks)
		}
		outsideClicks = nil
		// The next opening starts again from the next-up item, unfiltered.
		app.inbox?.presentationChanged(isPresented: false)
	}

	/// Hangs the panel under the status item at its content's height, before
	/// it shows: the place is set once, never animated, and holds until the
	/// panel closes.
	private func placePanel() {
		// `NSScreen.main` is the key window's screen, nil while the app runs in
		// the background with no key window.
		guard let screen = item.button?.window?.screen ?? NSScreen.main ?? NSScreen.screens.first else { return }
		let anchor = item.button?.window?.frame ?? .zero
		let natural = naturalHeight()
		let frame = InboxPanel.frame(
			size: NSSize(width: Tokens.popoverWidth, height: natural), under: anchor, in: screen.visibleFrame,
		)
		let maxHeight = InboxPanel.maxHeight(under: anchor, in: screen.visibleFrame)
		placement = (frame.minX, frame.maxY, maxHeight)
		layout.isCapped = natural > maxHeight
		height.reset(to: frame.height)
		panel.setFrame(frame, display: false, animate: false)
	}

	/// A display was added, removed or rearranged while the panel shows: the
	/// cap follows the screen the panel is on. Its place stays.
	@objc private func screensChanged(_: Notification) {
		guard panel.isVisible, let placement, let screen = panel.screen ?? NSScreen.main else { return }
		let maxHeight = placement.top - screen.visibleFrame.minY - InboxPanel.gap
		self.placement = (placement.x, placement.top, maxHeight)
		fitPanel(natural: layout.isCapped ? naturalHeight() : layout.naturalHeight)
	}

	/// The content's natural height, header and foot included, measured off
	/// screen.
	private func naturalHeight() -> CGFloat {
		ceil(measuringHost.sizeThatFits(in: NSSize(width: Tokens.popoverWidth, height: 100_000)).height)
	}

	/// While the content scrolls it reports no natural height of its own, so
	/// each change is measured off screen to see whether it fits again.
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
				if self.layout.isCapped {
					self.fitPanel(natural: self.naturalHeight())
				}
				self.fitOnChange()
			}
		}
	}

	/// The content's natural height changed, in the layout pass that starts
	/// its animation: the panel's height starts towards it at the next
	/// display frame, the first the animated content can be drawn in, on the
	/// same curve, so the two move together (a stalled main thread delays
	/// both alike). Taller than the screen, the panel stops there and the
	/// content scrolls.
	private func fitPanel(natural: CGFloat) {
		guard panel.isVisible, let placement else { return }
		let natural = ceil(natural)
		layout.isCapped = natural > placement.maxHeight
		let target = min(natural, placement.maxHeight)
		guard target != height.target || pendingHeight != nil else { return }
		pendingHeight = target
		guard heightLink == nil else { return }
		let link = panel.displayLink(target: self, selector: #selector(stepHeight(_:)))
		link.add(to: .main, forMode: .common)
		heightLink = link
	}

	/// One display frame of the height motion: the top edge and the left edge
	/// stay; only the bottom edge moves. Retargets and frames share one
	/// clock, the time of the frame.
	@objc private func stepHeight(_ link: CADisplayLink) {
		guard let placement else { return }
		let time = CACurrentMediaTime()
		if let pendingHeight {
			height.retarget(to: pendingHeight, at: time)
			self.pendingHeight = nil
		}
		let current = height.height(at: time)
		panel.setFrame(
			NSRect(x: placement.x, y: placement.top - current, width: Tokens.popoverWidth, height: current),
			display: true, animate: false,
		)
		if !height.isMoving(at: time) {
			link.invalidate()
			heightLink = nil
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
				// After the motion settles.
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
