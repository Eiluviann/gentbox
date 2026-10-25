import AppKit
import GentboxCore

/// A part of the inbox above, around or below the list: it lays itself out
/// for a width and says how tall it is.
@MainActor
protocol InboxSection: NSView {
	func layOut(width: CGFloat) -> CGFloat
}

/// "Inbox", and the settings menu (Open at Login, Quit).
@MainActor
final class InboxHeaderView: FlippedView, InboxSection {
	private let title = Label.line("Inbox", size: 17, weight: .semibold)
	private let settings = NSButton()
	private let loginItem: LoginItem

	init(loginItem: LoginItem) {
		self.loginItem = loginItem
		super.init(frame: .zero)
		settings.isBordered = false
		settings.image = NSImage(systemSymbolName: "gearshape", accessibilityDescription: nil)?
			.withSymbolConfiguration(.init(pointSize: 13, weight: .regular))
		settings.imagePosition = .imageOnly
		settings.contentTintColor = .labelColor
		settings.target = self
		settings.action = #selector(showSettings)
		settings.setAccessibilityLabel("Settings")
		settings.setAccessibilityIdentifier("inbox.settings")
		addSubview(title)
		addSubview(settings)
	}

	func layOut(width: CGFloat) -> CGFloat {
		let row: CGFloat = 28
		let top: CGFloat = 12
		let titleSize = Label.size(of: title)
		title.frame = Label.frame(
			x: 14, y: top + ((row - titleSize.height) / 2).rounded(), width: Label.textWidth(of: title), height: titleSize.height,
		)
		settings.frame = NSRect(x: width - 12 - row, y: top, width: row, height: row)
		return top + row + 10
	}

	@objc private func showSettings() {
		loginItem.refresh()
		let menu = NSMenu()
		let login = ClosureMenuItem("Open at Login", symbol: nil) { [loginItem] in
			loginItem.setEnabled(!loginItem.isEnabled)
		}
		login.state = loginItem.isEnabled ? .on : .off
		menu.addItem(login)
		if loginItem.needsApproval {
			let approval = NSMenuItem(title: "Allow Gentbox in System Settings › Login Items", action: nil, keyEquivalent: "")
			approval.isEnabled = false
			menu.addItem(approval)
		}
		#if DEBUG || GENTBOX_HOOKS
		menu.addItem(ClosureMenuItem("Debug Panel…", symbol: nil) {
			DebugPanel.shared?.present()
		})
		#endif
		menu.addItem(.separator())
		menu.addItem(ClosureMenuItem("Quit Gentbox", symbol: nil, keyEquivalent: "q") {
			NSApp.terminate(nil)
		})
		menu.popUp(positioning: nil, at: NSPoint(x: 0, y: settings.bounds.height + 2), in: settings)
	}
}

/// A hairline between parts: SwiftUI's `Divider`, `--line`.
@MainActor
final class RuleView: FlippedView, InboxSection {
	override init(frame: NSRect) {
		super.init(frame: frame)
		fill = Tokens.line
	}

	func layOut(width _: CGFloat) -> CGFloat {
		1
	}
}

/// After Close found open items: "2 items still open in Phase 3", counted
/// live as they close, and Show all to leave the filter. At 0 the banner
/// offers the plain Close.
@MainActor
final class FilterBannerView: FlippedView, InboxSection {
	private let words = Label.line(size: 11, color: .secondaryLabelColor)
	private let close: InboxButton
	private let showAll: InboxButton

	init(inbox: InboxModel) {
		close = InboxButton(title: "Close thread", role: .primary, size: .small) {
			Task { await inbox.closeFilteredThread() }
		}
		showAll = InboxButton(title: "Show all", role: .link) {
			inbox.clearFilter()
		}
		super.init(frame: .zero)
		words.setAccessibilityIdentifier("inbox.filter")
		close.setAccessibilityIdentifier("inbox.close-filtered")
		showAll.setAccessibilityIdentifier("inbox.show-all")
		for view in [words, close, showAll] {
			addSubview(view)
		}
	}

	private var offersClose = false

	func show(title: String, count: Int) {
		words.stringValue = Self.words(count: count, title: title)
		offersClose = count == 0
		close.setConcealed(!offersClose)
	}

	func layOut(width: CGFloat) -> CGFloat {
		let height = max(Label.size(of: words).height, offersClose ? close.frame.height : 0, showAll.frame.height)
		let top: CGFloat = 8
		var right = width - 12
		// A borderless button pads its title like a label.
		showAll.frame.origin = NSPoint(
			x: right - showAll.frame.width + Label.inset, y: top + ((height - showAll.frame.height) / 2).rounded(),
		)
		right = showAll.frame.minX - 8
		if offersClose {
			close.frame.origin = NSPoint(x: right - close.frame.width, y: top + ((height - close.frame.height) / 2).rounded())
			right = close.frame.minX - 8
		}
		let wordsHeight = Label.size(of: words).height
		words.frame = Label.frame(
			x: Tokens.rowHorizontalPadding, y: top + ((height - wordsHeight) / 2).rounded(),
			width: max(right - Tokens.rowHorizontalPadding, 0), height: wordsHeight,
		)
		return top + height + 8
	}

	static func words(count: Int, title: String) -> String {
		switch count {
		case 0: "Nothing open in \(title)"
		case 1: "1 item still open in \(title)"
		default: "\(count) items still open in \(title)"
		}
	}
}

/// The foot: the picked rows' Dismiss, what is muted (with Unmute), and the
/// last failure.
@MainActor
final class InboxFooterView: FlippedView, InboxSection {
	struct Content: Equatable {
		var selectionCount = 0
		var failure: String?
		var muted: [MutedGroup] = []

		var isEmpty: Bool {
			selectionCount == 0 && failure == nil && InboxWords.mutedSummary(muted) == nil
		}
	}

	private let inbox: InboxModel
	private let selected = Label.line(size: 11, color: .secondaryLabelColor)
	private let cancel: InboxButton
	private let dismiss: InboxButton
	private let failureIcon = Symbol.view("xmark", size: 10, color: Tokens.waiting)
	private let failure = Label.line(size: 11, color: Tokens.waiting)
	private let mutedIcon = Symbol.view("bell.slash", size: 12, color: .secondaryLabelColor)
	private let muted = Label.line(size: 11, color: .secondaryLabelColor)
	private let unmute = NSButton()
	private var content = Content()

	init(inbox: InboxModel) {
		self.inbox = inbox
		cancel = InboxButton(title: "Cancel", role: .secondary, size: .small) { inbox.clearSelection() }
		dismiss = InboxButton(title: "Dismiss", role: .primary, size: .small) {
			Task { await inbox.dismissSelected() }
		}
		super.init(frame: .zero)
		dismiss.setAccessibilityIdentifier("inbox.dismiss-selected")
		failure.setAccessibilityIdentifier("inbox.failure")
		failureIcon.setAccessibilityElement(false)
		mutedIcon.setAccessibilityElement(false)
		muted.setAccessibilityIdentifier("inbox.muted-summary")
		unmute.isBordered = false
		unmute.attributedTitle = NSAttributedString(
			string: "Unmute", attributes: [.font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.controlAccentColor],
		)
		unmute.sizeToFit()
		unmute.target = self
		unmute.action = #selector(showUnmute)
		unmute.setAccessibilityIdentifier("inbox.unmute")
		for view in [selected, cancel, dismiss, failureIcon, failure, mutedIcon, muted, unmute] as [NSView] {
			addSubview(view)
		}
	}

	func show(_ content: Content) {
		self.content = content
		selected.stringValue = "\(content.selectionCount) selected"
		failure.stringValue = content.failure ?? ""
		muted.stringValue = InboxWords.mutedSummary(content.muted) ?? ""
	}

	func layOut(width: CGFloat) -> CGFloat {
		let left: CGFloat = 14
		let right = width - 12
		var y: CGFloat = 9
		var isFirst = true
		func next(_ height: CGFloat) -> CGFloat {
			if !isFirst { y += 6 }
			isFirst = false
			defer { y += height }
			return y
		}
		let hasSelection = content.selectionCount > 0
		for view in [selected, cancel, dismiss] as [NSView] {
			view.setConcealed(!hasSelection)
		}
		if hasSelection {
			let height = max(cancel.frame.height, dismiss.frame.height)
			let top = next(height)
			dismiss.frame.origin = NSPoint(x: right - dismiss.frame.width, y: top)
			cancel.frame.origin = NSPoint(x: dismiss.frame.minX - 8 - cancel.frame.width, y: top)
			let textHeight = Label.size(of: selected).height
			selected.frame = Label.frame(x: left, y: top + ((height - textHeight) / 2).rounded(), width: cancel.frame.minX - 8 - left, height: textHeight)
		}
		let showsFailure = content.failure != nil
		failure.setConcealed(!showsFailure)
		failureIcon.alphaValue = showsFailure ? 1 : 0
		if showsFailure {
			let height = Label.size(of: failure).height
			let top = next(height)
			let iconSize = failureIcon.image?.size ?? .zero
			failureIcon.frame = NSRect(x: left, y: top + ((height - iconSize.height) / 2).rounded(), width: iconSize.width, height: iconSize.height)
			failure.frame = Label.frame(x: failureIcon.frame.maxX + 6, y: top, width: right - failureIcon.frame.maxX - 6, height: height)
		}
		let showsMuted = InboxWords.mutedSummary(content.muted) != nil
		for view in [muted, unmute] as [NSView] {
			view.setConcealed(!showsMuted)
		}
		mutedIcon.alphaValue = showsMuted ? 1 : 0
		if showsMuted {
			let height = max(Label.size(of: muted).height, unmute.frame.height, mutedIcon.image?.size.height ?? 0)
			let top = next(height)
			let iconSize = mutedIcon.image?.size ?? .zero
			mutedIcon.frame = NSRect(x: left, y: top + ((height - iconSize.height) / 2).rounded(), width: iconSize.width, height: iconSize.height)
			unmute.frame.origin = NSPoint(x: right - unmute.frame.width + Label.inset, y: top + ((height - unmute.frame.height) / 2).rounded())
			let textHeight = Label.size(of: muted).height
			muted.frame = Label.frame(
				x: mutedIcon.frame.maxX + 8, y: top + ((height - textHeight) / 2).rounded(),
				width: unmute.frame.minX - 8 - mutedIcon.frame.maxX - 8, height: textHeight,
			)
		}
		return y + 9
	}

	@objc private func showUnmute() {
		let menu = NSMenu()
		for group in content.muted {
			menu.addItem(ClosureMenuItem(group.title, symbol: nil) { [inbox] in
				Task {
					switch group.kind {
					case .topic: await inbox.setTopicMuted(group.id, false)
					case .thread: await inbox.setThreadMuted(group.id, false)
					}
				}
			})
		}
		menu.popUp(positioning: nil, at: NSPoint(x: 0, y: unmute.bounds.height + 2), in: unmute)
	}
}

/// A centred symbol and a line or two, for starting, failure and empty.
@MainActor
final class InboxNoticeView: FlippedView, InboxSection {
	private let symbol = NSImageView()
	private let title = Label.line(size: 13, weight: .semibold)
	private let caption = Label.wrapping(font: .systemFont(ofSize: 11), color: .secondaryLabelColor)

	override init(frame: NSRect) {
		super.init(frame: frame)
		symbol.imageScaling = .scaleNone
		caption.alignment = .center
		for view in [symbol, title, caption] as [NSView] {
			view.setAccessibilityElement(false)
			addSubview(view)
		}
		setAccessibilityElement(true)
		setAccessibilityRole(.staticText)
		setAccessibilityIdentifier("inbox.notice")
	}

	func show(symbol name: String, title words: String, caption detail: String?) {
		Symbol.set(symbol, name, size: 28, color: .tertiaryLabelColor)
		title.stringValue = words
		caption.stringValue = detail ?? ""
		caption.isHidden = detail == nil
		setAccessibilityLabel([words, detail].compactMap { $0 }.joined(separator: ", "))
	}

	func layOut(width: CGFloat) -> CGFloat {
		let inner = width - 40
		var y: CGFloat = 36
		let symbolSize = symbol.image?.size ?? .zero
		symbol.frame = NSRect(x: ((width - symbolSize.width) / 2).rounded(), y: y, width: symbolSize.width, height: symbolSize.height)
		y += symbolSize.height + 4 + 4
		let titleSize = Label.size(of: title)
		let titleWidth = min(titleSize.width, inner)
		title.frame = NSRect(x: ((width - titleWidth) / 2).rounded(), y: y, width: titleWidth, height: titleSize.height)
		y += titleSize.height
		if !caption.isHidden {
			y += 4
			let height = Label.height(of: caption, width: inner)
			caption.frame = Label.frame(x: 20, y: y, width: inner, height: height)
			y += height
		}
		return y + 36
	}
}
