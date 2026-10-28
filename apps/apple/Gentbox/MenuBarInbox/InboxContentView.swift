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
/// AppKit views, laid out by hand, moved by one motion system (ADR 0013):
/// the card is a stack of sections, the list a stack of rows and each row a
/// stack of parts, every stack a `SlotStack`. Each change of the model is
/// read once and run as one change of `InboxTransition`: every stack, the
/// list's scroll and the window's card (`onCardChange`) take their targets
/// in one transaction and move there on one curve and clock. The render
/// server runs the motion, so the main thread does the change's own work in
/// one frame and nothing in the frames after it.
@MainActor
final class InboxContentView: FlippedView {
	/// How the card changes for the content.
	var onCardChange: ((CardChange) -> Void)?
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
	private let list: ListView
	private let scroller = ListScrollView()
	private let footRule = RuleView()
	private var foot: InboxFooterView?
	private let headerSlot = SlotView()
	private let noticeSlot = SlotView()
	private let bannerSlot = SlotView()
	private let listSlot = SlotView()
	private let footSlot = SlotView()
	private lazy var sections = SlotStack<Section>(in: self, width: Tokens.popoverWidth)
	private let rowStack: SlotStack<Int>
	private lazy var transition = makeTransition()
	/// The motion's clock when a test winds it; the render server's
	/// otherwise.
	private let motionClock: (any MotionClock)?
	/// The listed rows, by message.
	private var rows: [String: InboxRowView] = [:]
	/// Rows closing their place as they leave, by row: parked or discarded
	/// once their change ends.
	private var leavingRows: [Int: InboxRowView] = [:]
	/// Rows of open messages a filter hides, hidden in the list, ready to
	/// come back.
	private var parkedRows: [String: InboxRowView] = [:]
	private var snapshot = Snapshot()
	/// What `prepareAhead` has queued: row ids, "banner", "foot".
	private var preparing: Set<String> = []
	private var hasApplied = false
	private var hasWarmedCheckboxes = false
	private var minuteTimer: Timer?
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
		/// The message the reply box is open on; its text is the field's.
		var replyingTo: String?
		var filter: Filter?
		var foot = InboxFooterView.Content()
		var moveTargets: [TopicChoice] = []
	}

	private enum Section: Hashable {
		case header
		case notice
		case banner
		case list
		case foot
	}

	/// Which row shows each listed message, worked out before a change's
	/// clock starts: making a row lays it out, and the change's first frame
	/// must not wait on that.
	private struct RowClaims {
		struct Claim {
			let messageId: String
			let row: InboxRowView
			let content: RowContent
		}

		/// In the list's order.
		let listed: [Claim]
		/// Rows no listed message took, by message.
		let unclaimed: [String: InboxRowView]
	}

	init(app: AppModel, clock: (any MotionClock)? = nil) {
		self.app = app
		motionClock = clock
		header = InboxHeaderView(loginItem: app.loginItem)
		let list = ListView()
		self.list = list
		rowStack = SlotStack(in: list, width: Tokens.popoverWidth)
		super.init(frame: NSRect(x: 0, y: 0, width: Tokens.popoverWidth, height: 0))
		rowStack.onDrop = { [weak self] rowId, _ in self?.rowDropped(rowId) }
		headerSlot.content.addSubview(header)
		headerSlot.content.addSubview(headerRule)
		noticeSlot.content.addSubview(notice)
		bannerSlot.content.addSubview(bannerRule)
		scroller.documentView = list
		listSlot.content.addSubview(scroller)
		footSlot.content.addSubview(footRule)
		// Shown as the section stack lists them.
		for slot in [headerSlot, noticeSlot, bannerSlot, listSlot, footSlot] {
			slot.isHidden = true
			addSubview(slot)
		}
		#if DEBUG || GENTBOX_HOOKS
		if AnimationTraceMarker.isShown {
			addSubview(traceMarker)
			traceMarker.layer?.zPosition = 10000
		}
		#endif
		_ = transition
		observe()
	}

	/// A row's card changed height by itself (the reply box took a line or
	/// gave one back): the same snapshot, laid out again with motion.
	private func refresh(_ row: InboxRowView) {
		apply(snapshot, animated: hasApplied && window?.isVisible == true, refreshing: row)
	}

	/// Lays the content out at once, with no motion: the height on opening.
	func settle() {
		apply(snapshot, animated: false)
	}

	/// The minute changed: rows say "4 min ago".
	private func startMinuteTimer() {
		minuteTimer?.invalidate()
		minuteTimer = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in
			MainActor.assumeIsolated {
				self?.rows.values.forEach { $0.refreshTime() }
			}
		}
	}

	override func viewDidMoveToWindow() {
		super.viewDidMoveToWindow()
		if window == nil {
			minuteTimer?.invalidate()
			minuteTimer = nil
		} else if minuteTimer == nil {
			startMinuteTimer()
		}
	}

	override func accessibilityChildren() -> [Any]? {
		withoutLeavingSlots(super.accessibilityChildren(), of: self)
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
			snapshot.replyingTo = inbox.replyingTo
			snapshot.filter = inbox.filter.map { Snapshot.Filter(title: $0.title, openCount: inbox.filterOpenCount) }
			snapshot.foot = InboxFooterView.Content(
				selectionCount: inbox.selection.count, failure: inbox.failure, muted: inboxSnapshot.muted,
			)
			snapshot.moveTargets = inboxSnapshot.moveTargets
		}
		return snapshot
	}

	// MARK: Applying a change

	private func apply(_ next: Snapshot, animated: Bool, refreshing refreshed: InboxRowView? = nil) {
		guard refreshed != nil || next != snapshot || !hasApplied || !animated else { return }
		let isOpenCardNew = next.expandedId != snapshot.expandedId
		snapshot = next
		hasApplied = true
		showSectionContent()
		let claims = claimRows()
		if animated {
			let duration = Tokens.motionDuration
			transition.run(for: duration) { change, time in
				let motion = MotionChange(
					change: change, time: time, duration: duration, reducesMotion: StatusItemController.reducesMotion,
				)
				move(claims, as: motion, showingOpenCard: isOpenCardNew, refreshing: refreshed)
			}
			IdleWork.holdOff(for: duration)
		} else {
			transition.settle()
			CATransaction.begin()
			CATransaction.setDisableActions(true)
			move(claims, as: nil, showingOpenCard: isOpenCardNew, refreshing: refreshed)
			CATransaction.commit()
		}
		prepareAhead()
	}

	/// Moves every stack, the list's scroll and the card to the snapshot, as
	/// change `motion`, or at once without one.
	private func move(
		_ claims: RowClaims,
		as motion: MotionChange?,
		showingOpenCard isOpenCardNew: Bool,
		refreshing refreshed: InboxRowView?,
	) {
		// Before any frame changes: a shorter document clamps the clip view.
		let previousOrigin = scroller.origin
		let anchor = ListScrollTarget.anchor(in: rowStack.timeline, origin: previousOrigin)
		updateRows(claims, in: motion)
		refreshed?.refreshLayout(in: motion)
		let shown = sectionsToShow()
		moveRows(as: motion, fadingContent: !isListHandedOver(to: shown))
		let sectionChange = moveSections(shown, as: motion) {
			self.fitList(from: previousOrigin, anchor: anchor, showingOpenCard: isOpenCardNew, as: motion)
		}
		#if DEBUG || GENTBOX_HOOKS
		moveTraceMarker(by: sectionChange?.heightTrack, as: motion)
		#endif
		reportCard(as: motion)
	}

	private func makeTransition() -> InboxTransition {
		let clock = motionClock ?? LayerMotionClock(on: layer)
		// The scroll ends its slides on the change's clock.
		scroller.now = { [clock] in clock.now }
		let transition = InboxTransition(clock: clock)
		transition.add(sections)
		transition.add(rowStack)
		transition.add(scroller)
		transition.onLastCompletion = { [weak self] in self?.finishMotion() }
		return transition
	}

	/// Every motion has run: the list's scroll geometry and the window,
	/// grown to hold every frame of it, shrink to the target.
	private func finishMotion() {
		if hasSlidBackIntoList() { return }
		onCardChange?(.motionEnded)
	}

	// MARK: Sections

	private func sectionsToShow() -> [Section] {
		switch snapshot.phase {
		case .starting, .failed, .reading:
			return [.header, .notice]
		case .running:
			var sections: [Section] = [.header]
			if snapshot.filter != nil, banner != nil { sections.append(.banner) }
			sections.append(snapshot.isEmpty ? .notice : .list)
			if !snapshot.foot.isEmpty, foot != nil { sections.append(.foot) }
			return sections
		}
	}

	/// What the notice, the banner and the foot say; a part, once made,
	/// stays, so showing it again costs nothing to build.
	private func showSectionContent() {
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
	}

	/// Whether the list comes or goes in this change: its section fades it
	/// as a whole then, and its rows come and go whole within it.
	private func isListHandedOver(to shown: [Section]) -> Bool {
		let isListShown = sections.timeline.slots[.list].map { $0.leavingChange == nil } ?? false
		return isListShown != shown.contains(.list)
	}

	/// Lays out every shown section's content and moves the section stack;
	/// the list is as tall as its rows, less what the cap cuts, and
	/// `fitList` lays it out once the sections have their frames.
	private func moveSections(
		_ shown: [Section],
		as motion: MotionChange?,
		fitList: () -> Void,
	) -> MotionTimeline<Section>.Change? {
		let heights = shown.map { (section: $0, height: layOutContent(of: $0)) }
		naturalHeight = heights.reduce(0) { $0 + $1.height }
		isCapped = shown.contains(.list) && naturalHeight > maxHeight
		let overflow = isCapped ? naturalHeight - maxHeight : 0
		let entries = heights.map { section, height in
			SlotStack<Section>.Entry(
				id: section, view: slot(of: section), height: section == .list ? max(height - overflow, 0) : height,
			)
		}
		let layOutList = shown.contains(.list) ? fitList : {}
		guard let motion else {
			sections.settle(entries)
			layOutList()
			return nil
		}
		return sections.apply(
			entries, change: motion.change, at: motion.time, duration: motion.duration, reducesMotion: motion.reducesMotion,
			layOut: layOutList,
		)
	}

	/// Lays out a section's content from its slot's top and returns its
	/// natural height; the list's is its rows'.
	private func layOutContent(of section: Section) -> CGFloat {
		let width = Tokens.popoverWidth
		switch section {
		case .header:
			let height = header.layOut(width: width)
			header.frame = NSRect(x: 0, y: 0, width: width, height: height)
			headerRule.frame = NSRect(x: 0, y: height, width: width, height: 1)
			headerSlot.content.frame = NSRect(x: 0, y: 0, width: width, height: height + 1)
			return height + 1
		case .notice:
			let height = notice.layOut(width: width)
			notice.frame = NSRect(x: 0, y: 0, width: width, height: height)
			noticeSlot.content.frame = notice.frame
			return height
		case .banner:
			let height = banner?.layOut(width: width) ?? 0
			banner?.frame = NSRect(x: 0, y: 0, width: width, height: height)
			bannerRule.frame = NSRect(x: 0, y: height, width: width, height: 1)
			bannerSlot.content.frame = NSRect(x: 0, y: 0, width: width, height: height + 1)
			return height + 1
		case .list:
			return rowStack.targetHeight
		case .foot:
			let height = foot?.layOut(width: width) ?? 0
			footRule.frame = NSRect(x: 0, y: 0, width: width, height: 1)
			foot?.frame = NSRect(x: 0, y: 1, width: width, height: height)
			footSlot.content.frame = NSRect(x: 0, y: 0, width: width, height: height + 1)
			return height + 1
		}
	}

	private func slot(of section: Section) -> SlotView {
		switch section {
		case .header: headerSlot
		case .notice: noticeSlot
		case .banner: bannerSlot
		case .list: listSlot
		case .foot: footSlot
		}
	}

	private func reportCard(as motion: MotionChange?) {
		let height = sections.targetHeight
		guard let motion else {
			onCardChange?(.settled(height: height))
			return
		}
		let tallest = max(sections.timeline.tallest(from: motion.time), height)
		onCardChange?(.moving(CardMotion(
			height: height, tallest: tallest, change: motion.change, start: motion.time, duration: motion.duration,
		)))
	}

	private func makeBanner() -> FilterBannerView? {
		if let banner { return banner }
		guard let inbox = app.inbox else { return nil }
		let banner = FilterBannerView(inbox: inbox)
		self.banner = banner
		bannerSlot.content.addSubview(banner)
		return banner
	}

	private func makeFoot() -> InboxFooterView? {
		if let foot { return foot }
		guard let inbox = app.inbox else { return nil }
		let foot = InboxFooterView(inbox: inbox)
		self.foot = foot
		footSlot.content.addSubview(foot)
		return foot
	}

	// MARK: Rows

	/// Rows that stay take a listed message; a row whose message was
	/// superseded takes its successor, so the card follows it in place; a
	/// leaving or parked row listed again comes back; new rows are made.
	private func claimRows() -> RowClaims {
		var unclaimed = parkedRows
		for row in leavingRows.values {
			unclaimed[row.messageId] = row
		}
		unclaimed.merge(rows) { _, listed in listed }
		var listed: [RowClaims.Claim] = []
		for (index, message) in snapshot.items.enumerated() {
			let content = RowContent(
				message: message,
				isExpanded: snapshot.expandedId == message.id,
				isPicked: snapshot.selection.contains(message.id),
				isSelecting: !snapshot.selection.isEmpty,
				isFirst: index == 0,
				isReplying: snapshot.replyingTo == message.id,
				moveTargets: snapshot.moveTargets,
			)
			let predecessor = message.supersedes.first { unclaimed[$0] != nil && !snapshot.openIds.contains($0) }
			let claimed = unclaimed.removeValue(forKey: message.id) ?? predecessor.flatMap { unclaimed.removeValue(forKey: $0) }
			guard let row = claimed ?? makeRow(content) else { continue }
			listed.append(RowClaims.Claim(messageId: message.id, row: row, content: content))
		}
		return RowClaims(listed: listed, unclaimed: unclaimed)
	}

	/// Claimed rows take their new content. A listed row no message claimed
	/// starts to leave, and a parked one whose message closed goes.
	private func updateRows(_ claims: RowClaims, in motion: MotionChange?) {
		for claim in claims.listed where claim.row.rowContent != claim.content {
			claim.row.apply(claim.content, in: motion)
		}
		let listed = Dictionary(claims.listed.map { ($0.messageId, $0.row) }) { _, later in later }
		let listedRowIds = Set(listed.values.map(\.rowId))
		leavingRows = leavingRows.filter { !listedRowIds.contains($0.key) }
		parkedRows = parkedRows.filter { !listedRowIds.contains($0.value.rowId) }
		for (messageId, row) in claims.unclaimed {
			if rows[messageId] === row {
				leavingRows[row.rowId] = row
			} else if parkedRows[messageId] === row, !snapshot.openIds.contains(messageId) {
				parkedRows[messageId] = nil
				discard(row)
			}
		}
		rows = listed
	}

	/// A new row, hidden until the row stack shows it.
	private func makeRow(_ content: RowContent) -> InboxRowView? {
		guard let inbox = app.inbox else { return nil }
		let row = InboxRowView(content: content, inbox: inbox)
		row.isHidden = true
		row.onBodyChange = { [weak self] row in self?.refresh(row) }
		transition.add(row)
		return row
	}

	private func moveRows(as motion: MotionChange?, fadingContent fadesContent: Bool) {
		let entries = snapshot.items.compactMap { rows[$0.id] }.map {
			SlotStack<Int>.Entry(id: $0.rowId, view: $0, height: $0.height)
		}
		guard let motion else {
			rowStack.settle(entries)
			return
		}
		rowStack.apply(
			entries, change: motion.change, at: motion.time, duration: motion.duration, reducesMotion: motion.reducesMotion,
			fadesContent: fadesContent,
		)
	}

	/// A row finished leaving: parked while its message is open, so a filter
	/// cleared brings it back, gone otherwise.
	private func rowDropped(_ rowId: Int) {
		guard let row = leavingRows.removeValue(forKey: rowId) else { return }
		row.settle()
		if snapshot.openIds.contains(row.messageId) {
			parkedRows[row.messageId] = row
		} else {
			discard(row)
		}
	}

	/// A row that is gone for good: hidden at once, taken out of the view
	/// tree in a quiet turn. Tearing down a row's native controls costs a
	/// frame's worth.
	private func discard(_ row: InboxRowView) {
		row.isHidden = true
		IdleWork.enqueue { [weak row] in row?.removeFromSuperview() }
	}

	// MARK: The list's scroll

	/// Sizes the scroll view to the tallest the list shows during the motion
	/// and the document to hold every row and the target offset, then
	/// scrolls there: kept from the anchor's place, and showing the open card
	/// whole when it is a different one now (ADR 0013, Anchor).
	private func fitList(
		from previousOrigin: CGFloat,
		anchor: (id: Int, top: CGFloat)?,
		showingOpenCard isOpenCardNew: Bool,
		as motion: MotionChange?,
	) {
		let listHeight = sections.timeline.slots[.list]?.height ?? 0
		let openRowId = isOpenCardNew ? snapshot.expandedId.flatMap { rows[$0]?.rowId } : nil
		let target = isCapped
			? ListScrollTarget.origin(
				from: previousOrigin, anchor: anchor, in: rowStack.timeline, showing: openRowId, viewHeight: listHeight,
			)
			: 0
		let scrollerHeight = motion.map { max(sections.timeline.tallest(of: .list, from: $0.time), listHeight) } ?? listHeight
		let rowsHeight = motion.map { max(rowStack.timeline.tallest(from: $0.time), rowStack.targetHeight) } ?? rowStack.targetHeight
		setListFrames(scrollerHeight: scrollerHeight, documentHeight: max(rowsHeight, target + scrollerHeight))
		scroller.verticalScrollElasticity = isCapped ? .automatic : .none
		scroller.scroll(from: previousOrigin, to: target, as: motion)
	}

	/// Shrinks the list's scroll geometry to its target once every motion
	/// has run. A scroll of the user's into the room the motion made slides
	/// back first, as a change of its own: whether it did.
	private func hasSlidBackIntoList() -> Bool {
		guard let listHeight = sections.timeline.slots[.list]?.height else { return false }
		let origin = scroller.origin
		if let lastOrigin = ListScrollTarget.slideBack(from: origin, rowsHeight: rowStack.targetHeight, viewHeight: listHeight) {
			let duration = Tokens.motionDuration
			transition.run(for: duration) { [scroller] change, time in
				let motion = MotionChange(
					change: change, time: time, duration: duration, reducesMotion: StatusItemController.reducesMotion,
				)
				scroller.scroll(from: origin, to: lastOrigin, as: motion)
			}
			return true
		}
		CATransaction.begin()
		CATransaction.setDisableActions(true)
		setListFrames(scrollerHeight: listHeight, documentHeight: max(rowStack.targetHeight, origin + listHeight))
		CATransaction.commit()
		return false
	}

	private func setListFrames(scrollerHeight: CGFloat, documentHeight: CGFloat) {
		let width = Tokens.popoverWidth
		list.frame = NSRect(x: 0, y: 0, width: width, height: documentHeight)
		scroller.frame = NSRect(x: 0, y: 0, width: width, height: scrollerHeight)
		listSlot.content.frame = scroller.frame
	}

	// MARK: Reading ahead

	/// Builds, in quiet turns, what a click would otherwise build in its
	/// frame: the banner, the foot and every row's card body.
	private func prepareAhead() {
		if !hasWarmedCheckboxes {
			hasWarmedCheckboxes = true
			IdleWork.enqueue { Symbol.warm(InboxRowView.checkboxSymbols, size: InboxRowView.checkboxSize) }
		}
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
		for message in likelyNextCards {
			guard let row = rows[message.id], !row.hasBody, !preparing.contains(message.id) else { continue }
			preparing.insert(message.id)
			IdleWork.enqueue { [weak self, weak row] in
				self?.preparing.remove(message.id)
				row?.prepareBody()
			}
		}
	}

	/// The cards a click or a hand-over can show next: the rows on screen
	/// (as far as the card reaches) and the next-up item. Rows scrolled out
	/// of reach build theirs when they are clicked.
	private var likelyNextCards: [InboxMessage] {
		var reach = min(maxHeight, NSScreen.main?.visibleFrame.height ?? maxHeight)
		var cards: [InboxMessage] = []
		for message in snapshot.items {
			guard let row = rows[message.id], reach > 0 else { break }
			cards.append(message)
			reach -= row.height
		}
		if let nextUp = app.inbox?.inbox?.nextUp, !cards.contains(where: { $0.id == nextUp.id }) {
			cards.append(nextUp)
		}
		return cards
	}

	private func isPreparing(banner: Bool) -> Bool {
		preparing.contains(banner ? "banner" : "foot")
	}

	// MARK: Tracing

	#if DEBUG || GENTBOX_HOOKS
	/// The marker sits just above the content's bottom and moves with the
	/// section stack's height, so a trace reads where the content ends.
	private func moveTraceMarker(by heightTrack: MotionTrack?, as motion: MotionChange?) {
		guard traceMarker.superview === self, let layer = traceMarker.layer else { return }
		traceMarker.frame = NSRect(
			x: 0, y: sections.targetHeight - AnimationTraceMarker.height - AnimationTraceMarker.thickness,
			width: Tokens.popoverWidth, height: AnimationTraceMarker.thickness,
		)
		guard let motion else {
			layer.removeMotions(keyedFrom: "motion.marker.")
			return
		}
		guard let heightTrack, heightTrack.delta != 0 else { return }
		layer.addMotion(heightTrack, keyPath: "position.y", key: "motion.marker.\(motion.change)")
	}

	/// Where the content's bottom shows now, mid-motion, in points below its
	/// top: from the trace marker's presented place. Worked out between
	/// presentation layers, which convert alike on every macOS, whereas a
	/// bitmap of the flipped tree comes out cut differently on macOS 15.
	var presentedContentBottom: CGFloat? {
		guard traceMarker.superview === self, let root = layer, let marker = traceMarker.layer, let top = headerSlot.layer
		else { return nil }
		let space = root.presentation() ?? root
		let shownMarker = (marker.presentation() ?? marker).convert(marker.bounds, to: space)
		let shownHeader = (top.presentation() ?? top).convert(top.bounds, to: space)
		// The header is at the top, whichever way the layer's y runs.
		if shownHeader.midY < shownMarker.midY {
			return shownMarker.maxY + AnimationTraceMarker.height - space.bounds.minY
		}
		return space.bounds.maxY - (shownMarker.minY - AnimationTraceMarker.height)
	}
	#endif
}

/// The rows; those leaving are left out for VoiceOver.
private final class ListView: FlippedView {
	override func accessibilityChildren() -> [Any]? {
		withoutLeavingSlots(super.accessibilityChildren(), of: self)
	}
}

/// `children` without what a slot of `view` that is leaving holds: out of
/// reach of VoiceOver, as of clicks, while it closes its place. Hidden
/// slots, parked or dropped, are out of the tree already.
@MainActor
private func withoutLeavingSlots(_ children: [Any]?, of view: NSView) -> [Any]? {
	let leavingSlots = view.subviews.compactMap { $0 as? SlotView }.filter(\.isLeaving)
	guard !leavingSlots.isEmpty else { return children }
	return children?.filter { child in
		guard let childView = child as? NSView else { return true }
		return !leavingSlots.contains { childView.isDescendant(of: $0) }
	}
}
