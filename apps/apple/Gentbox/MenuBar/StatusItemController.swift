import AppKit
import GentboxCore
import Observation
import SwiftUI

/// The menu-bar item and its popover: the Gentbox mark, the count of agents
/// waiting on you, and the inbox in a transient popover.
@MainActor
final class StatusItemController: NSObject, NSPopoverDelegate {
	/// What the UI tests look the item up by.
	static let accessibilityIdentifier = "gentbox.status-item"

	private let app: AppModel
	private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
	private let popover = NSPopover()
	/// The UI tests' window (see `showInboxWindowForTests`).
	private var testWindow: NSWindow?

	init(app: AppModel) {
		self.app = app
		super.init()
		configureButton()
		configurePopover()
		updateButton()
		if UserDefaults.standard.bool(forKey: "GentboxInboxInWindow") {
			showInboxWindowForTests()
		}
	}

	/// A test hook: the same inbox view in an ordinary window. With "hide the
	/// menu bar automatically" on, a status item stays above the screen for
	/// XCUITest, whose synthetic pointer never reveals it.
	private func showInboxWindowForTests() {
		// A fixed size: letting the content size the window feeds its frame
		// back into SwiftUI's layout, which AppKit aborts as a loop.
		let window = NSWindow(
			contentRect: NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: 720),
			styleMask: [.titled, .closable],
			backing: .buffered,
			defer: false,
		)
		window.contentView = NSHostingView(rootView: MenuBarInbox(app: app).frame(maxHeight: .infinity, alignment: .top))
		window.title = "Gentbox Inbox"
		window.center()
		window.makeKeyAndOrderFront(nil)
		NSApp.activate()
		testWindow = window
		if let path = ProcessInfo.processInfo.environment["GENTBOX_SNAPSHOT_PATH"], !path.isEmpty {
			// Snapshots are for comparing with the design boards, which are light.
			window.appearance = NSAppearance(named: .aqua)
			snapshotOnChange(to: URL(fileURLWithPath: path))
		}
	}

	/// A test hook: writes the test window's content to a PNG after every
	/// inbox change. The app draws its own window, so no Screen Recording
	/// permission is involved (a runner without it captures only the desktop
	/// picture behind the window).
	private func snapshotOnChange(to url: URL) {
		withObservationTracking {
			_ = app.inbox?.inbox
			_ = app.inbox?.failure
			_ = app.inbox?.selectedMessageId
		} onChange: { [weak self] in
			Task { @MainActor in
				try? await Task.sleep(for: .milliseconds(400))
				self?.writeSnapshot(to: url)
				self?.snapshotOnChange(to: url)
			}
		}
	}

	private func writeSnapshot(to url: URL) {
		guard let view = testWindow?.contentView,
			let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)
		else { return }
		view.cacheDisplay(in: view.bounds, to: bitmap)
		// The cache leaves the window's own background out: lay it under.
		let size = NSSize(width: bitmap.pixelsWide, height: bitmap.pixelsHigh)
		guard let opaque = NSBitmapImageRep(
			bitmapDataPlanes: nil, pixelsWide: bitmap.pixelsWide, pixelsHigh: bitmap.pixelsHigh,
			bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
			colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0,
		) else { return }
		NSGraphicsContext.saveGraphicsState()
		NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: opaque)
		view.effectiveAppearance.performAsCurrentDrawingAppearance {
			NSColor.windowBackgroundColor.setFill()
			NSRect(origin: .zero, size: size).fill()
		}
		bitmap.draw(in: NSRect(origin: .zero, size: size))
		NSGraphicsContext.restoreGraphicsState()
		try? opaque.representation(using: .png, properties: [:])?.write(to: url, options: .atomic)
	}

	private func configureButton() {
		guard let button = item.button else { return }
		button.imagePosition = .imageLeading
		button.font = .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .medium)
		button.setAccessibilityIdentifier(Self.accessibilityIdentifier)
		button.setAccessibilityLabel("Gentbox")
		button.target = self
		button.action = #selector(togglePopover(_:))
	}

	private func configurePopover() {
		popover.behavior = .transient
		popover.animates = false
		popover.delegate = self
		let host = NSHostingController(rootView: MenuBarInbox(app: app))
		host.sizingOptions = [.preferredContentSize]
		popover.contentViewController = host
	}

	@objc private func togglePopover(_ sender: NSStatusBarButton) {
		if popover.isShown {
			popover.performClose(sender)
		} else {
			app.inbox?.requestRead()
			popover.show(relativeTo: sender.bounds, of: sender, preferredEdge: .minY)
			popover.contentViewController?.view.window?.makeKey()
		}
	}

	/// The next opening starts again from the message an agent is blocked on.
	func popoverDidClose(_: Notification) {
		app.inbox?.selectedMessageId = nil
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
