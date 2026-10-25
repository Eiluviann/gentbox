#if DEBUG || GENTBOX_HOOKS
import AppKit
import GentboxCore

/// A small floating window for trying the inbox's motion by hand: it fills
/// the inbox from an in-memory playground instead of the hub, adds, changes,
/// replaces and withdraws messages, opens and closes the inbox, and slows
/// every motion down to look at it. Debug builds only; the settings menu
/// opens it, as does `-GentboxDebugPanel YES` at launch.
///
/// It never takes the keyboard: the inbox closes when its panel loses it,
/// and the inbox must stay open while you press buttons here.
@MainActor
final class DebugPanel: NSPanel {
	/// Made by the status item's controller; the settings menu shows it.
	private(set) static var shared: DebugPanel?
	static let openAtLaunchKey = "GentboxDebugPanel"

	private let app: AppModel
	private let openInbox: () -> Void
	private let closeInbox: () -> Void
	private let playground = PlaygroundInbox()
	private var isShowingPlayground = false
	/// The latest switch between hub and playground; an older one still
	/// filling the playground gives way to it.
	private var sourceSwitch: Task<Void, Never>?
	/// What this panel overrode, and so puts back on closing.
	private var hasOverriddenMotion = false
	private var hasOverriddenAppearance = false
	private let sourcePicker = FirstMouseSegmented(labels: ["Hub", "Playground"])
	private let kindPicker = FirstMousePopUp()
	private let speedPicker = FirstMousePopUp()
	private let motionPicker = FirstMouseSegmented(labels: ["System", "On", "Off"])
	private let appearancePicker = FirstMouseSegmented(labels: ["System", "Light", "Dark"])
	/// The buttons that change the playground, enabled while it shows.
	private var playgroundControls: [NSControl] = []

	private static let slowdowns: [Double] = [1, 2, 4, 8, 16]

	static func install(app: AppModel, openInbox: @escaping () -> Void, closeInbox: @escaping () -> Void) {
		let panel = DebugPanel(app: app, openInbox: openInbox, closeInbox: closeInbox)
		shared = panel
		if UserDefaults.standard.bool(forKey: openAtLaunchKey) {
			panel.present()
		}
	}

	private init(app: AppModel, openInbox: @escaping () -> Void, closeInbox: @escaping () -> Void) {
		self.app = app
		self.openInbox = openInbox
		self.closeInbox = closeInbox
		super.init(
			contentRect: .zero, styleMask: [.titled, .closable, .utilityWindow, .nonactivatingPanel],
			backing: .buffered, defer: true,
		)
		title = "Gentbox Debug"
		isFloatingPanel = true
		becomesKeyOnlyIfNeeded = true
		hidesOnDeactivate = false
		isReleasedWhenClosed = false
		collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
		contentView = makeContent()
		setContentSize(contentView?.fittingSize ?? .zero)
	}

	override var canBecomeKey: Bool { false }
	override var canBecomeMain: Bool { false }

	/// Shows the panel beside the inbox, on the playground, filled once.
	func present() {
		if !isVisible, let screen = NSScreen.screens.first {
			let visible = screen.visibleFrame
			let x = visible.maxX - Tokens.popoverWidth - 2 * InboxPanel.margin - frame.width - 12
			setFrameTopLeftPoint(NSPoint(x: x, y: visible.maxY - 8))
		}
		orderFrontRegardless()
		if !isShowingPlayground {
			sourcePicker.selectedSegment = 1
			showPlayground(true, fillingIfEmpty: true)
		}
	}

	/// Closing goes back to the hub's inbox, at real speed, as the Mac is set.
	override func close() {
		showPlayground(false, fillingIfEmpty: false)
		sourcePicker.selectedSegment = 0
		Tokens.motionScale = 1
		speedPicker.selectItem(at: 0)
		if hasOverriddenMotion {
			StatusItemController.reduceMotionOverride = nil
			motionPicker.selectedSegment = 0
			hasOverriddenMotion = false
		}
		if hasOverriddenAppearance {
			NSApp.appearance = nil
			appearancePicker.selectedSegment = 0
			hasOverriddenAppearance = false
		}
		super.close()
	}

	// MARK: Building

	private func makeContent() -> NSView {
		kindPicker.addItems(withTitles: PlaygroundInbox.Kind.allCases.map(\.rawValue))
		speedPicker.addItems(withTitles: Self.slowdowns.map { $0 == 1 ? "Real speed" : "\(Int($0))× slower" })
		sourcePicker.selectedSegment = 0
		motionPicker.selectedSegment = 0
		appearancePicker.selectedSegment = 0
		sourcePicker.onChange = { [weak self] index in self?.showPlayground(index == 1, fillingIfEmpty: true) }
		speedPicker.onChange = { [weak self] index in
			guard let self else { return }
			Tokens.motionScale = Self.slowdowns[safe: index] ?? 1
		}
		motionPicker.onChange = { [weak self] index in
			self?.hasOverriddenMotion = true
			StatusItemController.reduceMotionOverride = index == 0 ? nil : index == 1
		}
		appearancePicker.onChange = { [weak self] index in
			self?.hasOverriddenAppearance = true
			NSApp.appearance = index == 0 ? nil : NSAppearance(named: index == 1 ? .aqua : .darkAqua)
		}

		let add = button("Add") { [weak self] in
			guard let self else { return }
			let kind = PlaygroundInbox.Kind.allCases[safe: self.kindPicker.indexOfSelectedItem] ?? .decision
			self.change { await $0.add(kind) }
		}
		let addThree = button("Add 3") { [weak self] in
			self?.change { playground in
				for kind in [PlaygroundInbox.Kind.completed, .decision, .note] {
					await playground.add(kind)
				}
			}
		}
		let populate = button("Populate") { [weak self] in self?.change { await $0.populate() } }
		let clear = button("Clear") { [weak self] in self?.change { await $0.clear() } }
		let supersede = button("Replace") { [weak self] in self?.changeExpanded { await $0.supersede($1) } }
		let changeText = button("Change text") { [weak self] in self?.changeExpanded { await $0.changeText(of: $1) } }
		let waiter = button("Waiter on/off") { [weak self] in self?.changeExpanded { await $0.toggleWaiter(on: $1) } }
		let withdrawExpanded = button("Withdraw") { [weak self] in self?.changeExpanded { await $0.withdraw($1) } }
		let withdrawLast = button("Withdraw last row") { [weak self] in
			guard let self, let inbox = self.app.inbox else { return }
			guard let last = inbox.visibleItems.last(where: { !inbox.isExpanded($0.id) }) else { return }
			self.change { await $0.withdraw(last.id) }
		}
		let withdrawNewest = button("Withdraw newest") { [weak self] in
			self?.change { playground in
				if let newest = await playground.newestId {
					await playground.withdraw(newest)
				}
			}
		}
		playgroundControls = [kindPicker, add, addThree, populate, clear, supersede, changeText, waiter, withdrawExpanded, withdrawLast, withdrawNewest]

		let rows: [NSView] = [
			row("Source", [sourcePicker]),
			row("Inbox", [button("Open") { [weak self] in self?.openInbox() }, button("Close") { [weak self] in self?.closeInbox() }]),
			row("List", [populate, clear]),
			row("Add", [kindPicker, add, addThree]),
			row("Open card", [supersede, changeText]),
			row("", [waiter, withdrawExpanded]),
			row("Other rows", [withdrawLast, withdrawNewest]),
			row("Motion", [speedPicker]),
			row("Reduce Motion", [motionPicker]),
			row("Appearance", [appearancePicker]),
		]
		let stack = NSStackView(views: rows)
		stack.orientation = .vertical
		stack.alignment = .leading
		stack.spacing = 8
		stack.edgeInsets = NSEdgeInsets(top: 12, left: 14, bottom: 14, right: 14)
		setPlaygroundControlsEnabled(false)
		return stack
	}

	private func row(_ title: String, _ controls: [NSView]) -> NSView {
		let label = NSTextField(labelWithString: title)
		label.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
		label.textColor = .secondaryLabelColor
		label.alignment = .right
		label.widthAnchor.constraint(equalToConstant: 92).isActive = true
		let row = NSStackView(views: [label] + controls)
		row.orientation = .horizontal
		row.spacing = 6
		return row
	}

	private func button(_ title: String, action: @escaping () -> Void) -> FirstMouseButton {
		let button = FirstMouseButton(title: title, onPress: action)
		button.bezelStyle = .push
		button.controlSize = .small
		button.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
		return button
	}

	// MARK: Changing the playground

	private func showPlayground(_ isShown: Bool, fillingIfEmpty: Bool) {
		isShowingPlayground = isShown
		setPlaygroundControlsEnabled(isShown)
		let playground = playground
		let previous = sourceSwitch
		previous?.cancel()
		sourceSwitch = Task {
			// One switch at a time, in the order they were made.
			await previous?.value
			guard !Task.isCancelled else { return }
			if isShown, fillingIfEmpty, await playground.inbox().items.isEmpty {
				await playground.populate()
			}
			guard !Task.isCancelled else { return }
			await app.show(isShown ? playground : nil)
		}
	}

	private func setPlaygroundControlsEnabled(_ isEnabled: Bool) {
		playgroundControls.forEach { $0.isEnabled = isEnabled }
	}

	/// Changes the playground, then has the inbox read it again, as a hub
	/// event would.
	private func change(_ edit: @escaping @Sendable (PlaygroundInbox) async -> Void) {
		let playground = playground
		Task {
			await edit(playground)
			app.inbox?.requestRead()
		}
	}

	/// Changes the expanded card's message.
	private func changeExpanded(_ edit: @escaping @Sendable (PlaygroundInbox, String) async -> Void) {
		guard let expanded = app.inbox?.expanded?.messageId else { return }
		change { await edit($0, expanded) }
	}
}

/// Controls that work on the first click in a window that never takes the
/// keyboard.
@MainActor
private final class FirstMouseButton: NSButton {
	private let onPress: () -> Void

	init(title: String, onPress: @escaping () -> Void) {
		self.onPress = onPress
		super.init(frame: .zero)
		self.title = title
		target = self
		action = #selector(press)
	}

	@available(*, unavailable)
	required init?(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	override func acceptsFirstMouse(for _: NSEvent?) -> Bool {
		true
	}

	@objc private func press() {
		onPress()
	}
}

@MainActor
private final class FirstMousePopUp: NSPopUpButton {
	var onChange: ((Int) -> Void)?

	init() {
		super.init(frame: .zero, pullsDown: false)
		controlSize = .small
		font = .systemFont(ofSize: NSFont.smallSystemFontSize)
		target = self
		action = #selector(changed)
	}

	@available(*, unavailable)
	required init?(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	override func acceptsFirstMouse(for _: NSEvent?) -> Bool {
		true
	}

	@objc private func changed() {
		onChange?(indexOfSelectedItem)
	}
}

@MainActor
private final class FirstMouseSegmented: NSSegmentedControl {
	var onChange: ((Int) -> Void)?

	init(labels: [String]) {
		super.init(frame: .zero)
		segmentCount = labels.count
		for (index, label) in labels.enumerated() {
			setLabel(label, forSegment: index)
		}
		trackingMode = .selectOne
		controlSize = .small
		font = .systemFont(ofSize: NSFont.smallSystemFontSize)
		target = self
		action = #selector(changed)
	}

	@available(*, unavailable)
	required init?(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	override func acceptsFirstMouse(for _: NSEvent?) -> Bool {
		true
	}

	@objc private func changed() {
		onChange?(selectedSegment)
	}
}

private extension Array {
	subscript(safe index: Int) -> Element? {
		indices.contains(index) ? self[index] : nil
	}
}
#endif
