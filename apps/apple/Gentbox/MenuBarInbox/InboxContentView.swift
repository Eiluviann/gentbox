import AppKit
import GentboxCore
import Observation

/// The menu-bar inbox (design: `mac-popover`): every open message on one
/// white surface with a rule between items, urgent first, then waiting, one
/// of them expanded into the full card. Topic and thread show only in each
/// row's breadcrumb; no section titles, no counters.
///
/// Three parts, as every Gentbox window: the header at the top, the content
/// at its natural height with no scrolling, and the foot at the bottom. The
/// card is as tall as the three, so it grows and shrinks with the content.
/// The one exception is content taller than the screen: the card stops at
/// `maxHeight` and only then does the list scroll, between the header and
/// the foot.
///
/// AppKit views, laid out by hand, moved by Core Animation: each change of
/// the model is read once, applied to the views that it changed, and every
/// view that moves or resizes animates from where it shows to its new place
/// on one curve, together with the card's height (`onHeightChange`). The
/// render server runs the motion, so the main thread does the change's own
/// work in one frame and nothing in the frames after it.
@MainActor
final class InboxContentView: FlippedView {
	/// The card's height for the content, and whether to animate to it.
	var onHeightChange: ((CGFloat, Bool) -> Void)?
	/// The tallest the card gets: below the status item down to the bottom
	/// of the screen.
	var maxHeight: CGFloat = .greatestFiniteMagnitude
	/// The content's height uncapped, header and foot included.
	private(set) var naturalHeight: CGFloat = 0
	/// Whether the content is taller than `maxHeight`, so the list scrolls.
	private(set) var isCapped = false

	private let app: AppModel
	private let header: InboxHeaderView
	private let headerRule = RuleView()
	private let notice = InboxNoticeView()
	private var banner: FilterBannerView?
	private let bannerRule = RuleView()
	private let list = ListView()
	private let scroller = NSScrollView()
	private let footRule = RuleView()
	private var foot: InboxFooterView?
	private var rows: [String: InboxRowView] = [:]
	/// Rows of open messages a filter hides.
	private var parkedRows: [String: InboxRowView] = [:]
	private var snapshot = Snapshot()
	private var shownSections: Set<Section> = []
	/// Parts fading out, parked once the fade ends.
	private var leaving: Set<ObjectIdentifier> = []
	/// What `prepareAhead` has queued: row ids, "banner", "foot".
	private var preparing: Set<String> = []
	private var hasApplied = false
	private var clock: Timer?
	#if DEBUG || GENTBOX_HOOKS
	private let traceMarker = AnimationTraceMarker()
	#endif

	/// What the views show: read from the model once per change.
	private struct Snapshot: Equatable {
		enum Phase: Equatable {
			case starting
			case failed(String)
			case reading
			case running
		}

		struct Filter: Equatable {
			let title: String
			let openCount: Int
		}

		var phase = Phase.starting
		var items: [InboxMessage] = []
		/// Every open message, filtered out or not.
		var openIds: Set<String> = []
		var isEmpty = true
		var expandedId: String?
		var selection: Set<String> = []
		var replyDraft: InboxModel.ReplyDraft?
		var filter: Filter?
		var foot = InboxFooterView.Content()
		var moveTargets: [TopicChoice] = []
	}

	private enum Section: Hashable {
		case notice
		case banner
		case list
		case foot
	}

	init(app: AppModel) {
		self.app = app
		header = InboxHeaderView(loginItem: app.loginItem)
		super.init(frame: NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: 0))
		scroller.drawsBackground = false
		scroller.hasVerticalScroller = true
		scroller.autohidesScrollers = true
		scroller.scrollerStyle = .overlay
		scroller.contentView = FlippedClipView()
		for view in [header, headerRule] as [NSView] {
			addSubview(view)
		}
		#if DEBUG || GENTBOX_HOOKS
		if AnimationTraceMarker.isShown {
			addSubview(traceMarker)
		}
		#endif
		observe()
	}

	/// Lays the content out at once, with no motion: the height on opening.
	func settle() {
		apply(snapshot, animated: false)
	}

	/// The minute changed: rows say "4 min ago".
	private func startClock() {
		clock?.invalidate()
		clock = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in
			MainActor.assumeIsolated {
				self?.rows.values.forEach { $0.refreshTime() }
			}
		}
	}

	override func viewDidMoveToWindow() {
		super.viewDidMoveToWindow()
		if window == nil {
			clock?.invalidate()
			clock = nil
		} else if clock == nil {
			startClock()
		}
	}

	// MARK: Reading the model

	/// Reads the model and applies it, then waits for its next change: each
	/// change is read once, however many properties it touched.
	private func observe() {
		let snapshot = withObservationTracking { read() } onChange: { [weak self] in
			DispatchQueue.main.async {
				MainActor.assumeIsolated { self?.observe() }
			}
		}
		apply(snapshot, animated: hasApplied && window?.isVisible == true)
	}

	private func read() -> Snapshot {
		var snapshot = Snapshot()
		switch app.phase {
		case .starting:
			snapshot.phase = .starting
		case .failed(let reason):
			snapshot.phase = .failed(reason)
		case .running(let inbox):
			guard let inboxSnapshot = inbox.inbox else {
				snapshot.phase = .reading
				return snapshot
			}
			snapshot.phase = .running
			snapshot.items = inbox.visibleItems
			snapshot.openIds = Set(inboxSnapshot.items.map(\.id))
			snapshot.isEmpty = inbox.isEmpty
			snapshot.expandedId = inbox.expanded?.messageId
			snapshot.selection = inbox.selection
			snapshot.replyDraft = inbox.replyDraft
			snapshot.filter = inbox.filter.map { Snapshot.Filter(title: $0.title, openCount: inbox.filterOpenCount) }
			snapshot.foot = InboxFooterView.Content(
				selectionCount: inbox.selection.count, failure: inbox.failure, muted: inboxSnapshot.muted,
			)
			snapshot.moveTargets = inboxSnapshot.moveTargets
		}
		return snapshot
	}

	// MARK: Applying a change

	private func apply(_ next: Snapshot, animated: Bool) {
		guard next != snapshot || !hasApplied || !animated else { return }
		snapshot = next
		hasApplied = true
		// Captured either way: without motion, what still moves stops.
		var motion = LayerMotion()
		captureEverything(into: &motion)
		CATransaction.begin()
		CATransaction.setDisableActions(true)
		let sections = sectionsToShow()
		updateSections(sections, isAnimated: animated)
		updateRows(isAnimated: animated)
		let visibleHeight = layOut(sections)
		motion.commit(animated: animated)
		onHeightChange?(visibleHeight, animated)
		CATransaction.commit()
		prepareAhead()
		if animated, isCapped, let expanded = snapshot.expandedId.flatMap({ rows[$0] }) {
			// No anchor: the least scroll that shows it, none if it shows.
			DispatchQueue.main.asyncAfter(deadline: .now() + Tokens.motionDuration) {
				MainActor.assumeIsolated { _ = expanded.scrollToVisible(expanded.bounds) }
			}
		}
	}

	private func captureEverything(into motion: inout LayerMotion) {
		for view in sectionViews {
			motion.capture(view.layer, ["position"])
		}
		#if DEBUG || GENTBOX_HOOKS
		motion.capture(traceMarker.layer, ["position"])
		#endif
		for row in rows.values {
			row.capture(into: &motion)
		}
	}

	private func sectionsToShow() -> [Section] {
		switch snapshot.phase {
		case .starting, .failed, .reading:
			return [.notice]
		case .running:
			var sections: [Section] = []
			if snapshot.filter != nil { sections.append(.banner) }
			sections.append(snapshot.isEmpty ? .notice : .list)
			if !snapshot.foot.isEmpty { sections.append(.foot) }
			return sections
		}
	}

	/// Parts that come fade in where they land; parts that go fade out where
	/// they were. A part, once made, stays: one not shown is parked below the
	/// card, where its mask hides it, still drawn, so showing it again costs
	/// no drawing.
	private func updateSections(_ sections: [Section], isAnimated: Bool) {
		switch snapshot.phase {
		case .starting:
			notice.show(symbol: "bubble.left", title: "Starting Gentbox", caption: nil)
		case .failed(let reason):
			notice.show(symbol: "xmark", title: "Gentbox could not start", caption: reason)
		case .reading:
			notice.show(symbol: "tray", title: "Reading the inbox", caption: nil)
		case .running:
			notice.show(symbol: "tray", title: "No agent needs you", caption: nil)
		}
		if let filter = snapshot.filter {
			makeBanner()?.show(title: filter.title, count: filter.openCount)
		}
		if !snapshot.foot.isEmpty {
			makeFoot()?.show(snapshot.foot)
		}
		let wanted = Set(sections)
		for section in wanted.subtracting(shownSections) {
			for view in views(of: section) {
				show(view, isAnimated: isAnimated)
			}
		}
		for section in shownSections.subtracting(wanted) {
			for view in views(of: section) {
				hide(view, isAnimated: isAnimated)
			}
		}
		shownSections = wanted
	}

	private func views(of section: Section) -> [NSView] {
		switch section {
		case .notice: [notice]
		case .banner: [banner, bannerRule].compactMap { $0 }
		case .list: [isCapped ? scroller : list]
		case .foot: [footRule, foot].compactMap { $0 }
		}
	}

	/// Every part that can come and go, made or not.
	private var sectionViews: [NSView] {
		[notice, banner, bannerRule, list, scroller, footRule, foot].compactMap { $0 }
	}

	private var shownViews: [NSView] {
		shownSections.flatMap(views(of:))
	}

	private func show(_ view: NSView, isAnimated: Bool) {
		adopt(view)
		leaving.remove(ObjectIdentifier(view))
		if isAnimated {
			LayerMotion.fade(view.layer, to: 1, duration: Tokens.motionDuration, from: view.layer?.presentation()?.opacity ?? 0)
		} else {
			view.layer?.removeAnimation(forKey: "motion.opacity")
			view.layer?.opacity = 1
		}
	}

	private func hide(_ view: NSView, isAnimated: Bool) {
		guard isAnimated else {
			view.layer?.removeAnimation(forKey: "motion.opacity")
			view.layer?.opacity = 0
			park(view)
			return
		}
		let id = ObjectIdentifier(view)
		leaving.insert(id)
		LayerMotion.fade(view.layer, to: 0, duration: Tokens.motionDuration)
		DispatchQueue.main.asyncAfter(deadline: .now() + Tokens.motionDuration) { [weak self, weak view] in
			MainActor.assumeIsolated {
				guard let self, let view, self.leaving.remove(id) != nil else { return }
				self.park(view)
			}
		}
	}

	/// Puts a part in the content, parked and transparent.
	private func adopt(_ view: NSView) {
		guard view.superview !== self else { return }
		view.wantsLayer = true
		view.layer?.opacity = 0
		addSubview(view, positioned: .below, relativeTo: header)
		park(view)
	}

	/// Below the card's bottom edge, outside its mask and its clicks.
	private func park(_ view: NSView) {
		view.move(to: NSPoint(x: 0, y: naturalHeight + 64))
	}

	private func isParked(_ view: NSView) -> Bool {
		sectionViews.contains { $0 === view } && !shownViews.contains { $0 === view }
	}

	override func accessibilityChildren() -> [Any]? {
		super.accessibilityChildren()?.filter { child in
			guard let view = child as? NSView else { return true }
			return !isParked(view)
		}
	}

	private func makeBanner() -> FilterBannerView? {
		if let banner { return banner }
		guard let inbox = app.inbox else { return nil }
		let banner = FilterBannerView(inbox: inbox)
		self.banner = banner
		adopt(banner)
		adopt(bannerRule)
		return banner
	}

	private func makeFoot() -> InboxFooterView? {
		if let foot { return foot }
		guard let inbox = app.inbox else { return nil }
		let foot = InboxFooterView(inbox: inbox)
		self.foot = foot
		adopt(foot)
		adopt(footRule)
		return foot
	}

	/// Builds, in quiet turns, what a click would otherwise build in its
	/// frame: the banner, the foot and every row's card body.
	private func prepareAhead() {
		if app.inbox != nil, banner == nil, !isPreparing(banner: true) {
			preparing.insert("banner")
			IdleWork.enqueue { [weak self] in _ = self?.makeBanner() }
		}
		if app.inbox != nil, foot == nil, !isPreparing(banner: false) {
			preparing.insert("foot")
			IdleWork.enqueue { [weak self] in
				guard let self, let foot = self.makeFoot() else { return }
				foot.show(InboxFooterView.Content(selectionCount: 1, failure: "", muted: []))
				_ = foot.layOut(width: Tokens.popoverWidth)
			}
		}
		for message in snapshot.items {
			guard let row = rows[message.id], !row.hasBody, !preparing.contains(message.id) else { continue }
			preparing.insert(message.id)
			IdleWork.enqueue { [weak self, weak row] in
				self?.preparing.remove(message.id)
				row?.prepareBody()
			}
		}
	}

	private func isPreparing(banner: Bool) -> Bool {
		preparing.contains(banner ? "banner" : "foot")
	}

	/// Rows that stay take their new content; a row whose message was
	/// superseded shows its successor, so the card follows it in place; new
	/// rows fade in where they land. A row that leaves fades out where it
	/// was: for good once its message closed, or parked below the card while
	/// a filter hides it, ready to come back.
	private func updateRows(isAnimated: Bool) {
		guard let inbox = app.inbox else { return }
		var kept: [String: InboxRowView] = [:]
		var unclaimed = rows.merging(parkedRows) { visible, _ in visible }
		for (index, message) in snapshot.items.enumerated() {
			let content = RowContent(
				message: message,
				isExpanded: snapshot.expandedId == message.id,
				isPicked: snapshot.selection.contains(message.id),
				isFirst: index == 0,
				replyDraft: snapshot.replyDraft?.messageId == message.id ? snapshot.replyDraft : nil,
				moveTargets: snapshot.moveTargets,
			)
			let predecessor = message.supersedes.first { unclaimed[$0] != nil && !snapshot.openIds.contains($0) }
			if let row = unclaimed.removeValue(forKey: message.id) ?? predecessor.flatMap({ unclaimed.removeValue(forKey: $0) }) {
				let wasShown = rows.values.contains { $0 === row }
				if row.content != content {
					row.apply(content, isAnimated: isAnimated)
				}
				if !wasShown || row.isLeaving {
					bringBack(row, isAnimated: isAnimated)
				}
				kept[message.id] = row
			} else {
				let row = InboxRowView(content: content, inbox: inbox)
				list.addSubview(row)
				if isAnimated {
					LayerMotion.fade(row.layer, to: 1, duration: Tokens.motionDuration, from: 0)
				}
				kept[message.id] = row
			}
		}
		var parked: [String: InboxRowView] = [:]
		for (id, row) in unclaimed {
			let isOpen = snapshot.openIds.contains(id)
			if isOpen {
				parked[id] = row
			}
			let wasShown = rows[id] === row
			guard wasShown else { continue }
			send(row, away: !isOpen, isAnimated: isAnimated)
		}
		rows = kept
		parkedRows = parked
	}

	/// A row leaves: it fades out where it was, then goes for good or parks.
	private func send(_ row: InboxRowView, away isGone: Bool, isAnimated: Bool) {
		row.isLeaving = true
		row.setAccessibilityElement(false)
		let finish = { [weak self, weak row] in
			guard let self, let row, row.isLeaving else { return }
			if isGone {
				row.removeFromSuperview()
			} else {
				self.parkRow(row)
			}
		}
		guard isAnimated else {
			row.layer?.opacity = 0
			finish()
			return
		}
		LayerMotion.fade(row.layer, to: 0, duration: Tokens.motionDuration)
		DispatchQueue.main.asyncAfter(deadline: .now() + Tokens.motionDuration) {
			MainActor.assumeIsolated { finish() }
		}
	}

	/// A parked or leaving row shows again: it fades in at its place.
	private func bringBack(_ row: InboxRowView, isAnimated: Bool) {
		row.isLeaving = false
		row.setAccessibilityElement(true)
		if isAnimated {
			LayerMotion.fade(row.layer, to: 1, duration: Tokens.motionDuration, from: row.layer?.presentation()?.opacity ?? 0)
		} else {
			row.layer?.removeAnimation(forKey: "motion.opacity")
			row.layer?.opacity = 1
		}
	}

	/// Below the card, in the list, outside the card's mask and clicks.
	private func parkRow(_ row: InboxRowView) {
		row.move(to: NSPoint(x: 0, y: naturalHeight + 64 - list.frame.minY))
	}

	// MARK: Layout

	/// Places every part top to bottom and returns the card's height.
	private func layOut(_ sections: [Section]) -> CGFloat {
		let width = Tokens.popoverWidth
		var y: CGFloat = 0
		header.frame = NSRect(x: 0, y: y, width: width, height: header.layOut(width: width))
		y = header.frame.maxY
		headerRule.frame = NSRect(x: 0, y: y, width: width, height: 1)
		y += 1
		let listHeight = layOutRows()
		let fixedHeight = sectionsHeight(sections, width: width)
		naturalHeight = y + fixedHeight + (sections.contains(.list) ? listHeight : 0)
		let wasCapped = isCapped
		isCapped = naturalHeight > maxHeight
		if isCapped != wasCapped, sections.contains(.list) {
			swapScrolling()
		}
		for section in sections {
			switch section {
			case .notice:
				notice.frame = NSRect(x: 0, y: y, width: width, height: notice.layOut(width: width))
				y = notice.frame.maxY
			case .banner:
				guard let banner else { continue }
				banner.frame = NSRect(x: 0, y: y, width: width, height: banner.layOut(width: width))
				bannerRule.frame = NSRect(x: 0, y: banner.frame.maxY, width: width, height: 1)
				y = bannerRule.frame.maxY
			case .list:
				list.frame = NSRect(x: 0, y: isCapped ? 0 : y, width: width, height: listHeight)
				if isCapped {
					let shown = max(listHeight - (naturalHeight - maxHeight), 0)
					scroller.frame = NSRect(x: 0, y: y, width: width, height: shown)
					y += shown
				} else {
					y += listHeight
				}
			case .foot:
				guard let foot else { continue }
				footRule.frame = NSRect(x: 0, y: y, width: width, height: 1)
				foot.frame = NSRect(x: 0, y: y + 1, width: width, height: foot.layOut(width: width))
				y = foot.frame.maxY
			}
		}
		for view in sectionViews where isParked(view) && !leaving.contains(ObjectIdentifier(view)) && view.superview === self {
			park(view)
		}
		#if DEBUG || GENTBOX_HOOKS
		traceMarker.frame = NSRect(
			x: 0, y: y - AnimationTraceMarker.height - AnimationTraceMarker.thickness,
			width: width, height: AnimationTraceMarker.thickness,
		)
		#endif
		return y
	}

	/// The rows top to bottom; the list's height.
	private func layOutRows() -> CGFloat {
		var y: CGFloat = 0
		for message in snapshot.items {
			guard let row = rows[message.id] else { continue }
			row.move(to: NSPoint(x: 0, y: y))
			y += row.height
		}
		return y
	}

	/// The height of every part but the list.
	private func sectionsHeight(_ sections: [Section], width: CGFloat) -> CGFloat {
		sections.reduce(0) { height, section in
			switch section {
			case .notice: height + notice.layOut(width: width)
			case .banner: height + (banner?.layOut(width: width) ?? 0) + 1
			case .list: height
			case .foot: height + 1 + (foot?.layOut(width: width) ?? 0)
			}
		}
	}

	/// The list goes into the scroll view when the card stops at the screen,
	/// and back out when it fits again.
	private func swapScrolling() {
		if isCapped {
			list.removeFromSuperview()
			scroller.documentView = list
			adopt(scroller)
			scroller.layer?.opacity = 1
		} else {
			scroller.documentView = nil
			scroller.layer?.opacity = 0
			park(scroller)
			adopt(list)
			list.layer?.opacity = 1
		}
	}
}

/// A clip view whose document hangs from the top.
private final class FlippedClipView: NSClipView {
	override var isFlipped: Bool { true }
}

/// The rows; those leaving or parked are left out for VoiceOver.
private final class ListView: FlippedView {
	override func accessibilityChildren() -> [Any]? {
		super.accessibilityChildren()?.filter { ($0 as? InboxRowView)?.isLeaving != true }
	}
}
