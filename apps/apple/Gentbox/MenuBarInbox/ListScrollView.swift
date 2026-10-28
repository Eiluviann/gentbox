import AppKit
import QuartzCore

/// Where the list scrolls to for a change (ADR 0013, Anchor), worked out
/// from the row stack's targets alone, so the tests read it as the views do.
enum ListScrollTarget {
	/// Less than a pixel: a scroll's rounding, not a scroll past the end.
	private static let slideBackTolerance: CGFloat = 0.5

	/// The first row whose place reaches below the list's top at `origin`,
	/// and its top: the row that stays still on screen while rows above it,
	/// out of view, open and close. None at the top, where a row inserted
	/// above the others shows.
	static func anchor<ID: Hashable>(in timeline: MotionTimeline<ID>, origin: CGFloat) -> (id: ID, top: CGFloat)? {
		guard origin > 0 else { return nil }
		for id in timeline.order {
			guard let slot = timeline.slots[id], slot.top + slot.height > origin else { continue }
			return (id, slot.top)
		}
		return nil
	}

	/// The offset once the change has run: moved by the anchor's own change
	/// of top, then the least move from there that shows `openSlot` whole
	/// (its top, if it is taller than the view), clamped to the document.
	/// - Parameters:
	///   - origin: the offset before the change.
	///   - anchor: `anchor(in:origin:)` before the change.
	///   - timeline: the row stack after the change.
	///   - openSlot: the open card, when it is a different one now.
	///   - viewHeight: the height of the list once the change has run.
	static func origin<ID: Hashable>(
		from origin: CGFloat,
		anchor: (id: ID, top: CGFloat)?,
		in timeline: MotionTimeline<ID>,
		showing openSlot: ID?,
		viewHeight: CGFloat,
	) -> CGFloat {
		var target = origin
		if let anchor {
			target = timeline.slots[anchor.id].map { origin + $0.top - anchor.top } ?? origin
		}
		if let openSlot, let slot = timeline.slots[openSlot] {
			if slot.top < target {
				target = slot.top
			} else if slot.top + slot.height > target + viewHeight {
				target = min(slot.top + slot.height - viewHeight, slot.top)
			}
		}
		return min(max(target, 0), max(timeline.targetHeight - viewHeight, 0))
	}

	/// Where a scroll of the user's into the room a motion made slides back
	/// to once the motion has run: the list's end, when `origin` lies past
	/// it by more than a rounding; nil otherwise.
	static func slideBack(from origin: CGFloat, rowsHeight: CGFloat, viewHeight: CGFloat) -> CGFloat? {
		let lastOrigin = max(rowsHeight - viewHeight, 0)
		return origin > lastOrigin + slideBackTolerance ? lastOrigin : nil
	}
}

/// The list's scroll view (ADR 0013, Offset): a change scrolls the clip
/// view to its target at once, which AppKit, the scroller and hit-testing
/// read, and the document's layer slides from where it showed on the
/// change's clock, so the scroll and the rows move as one. A scroll of the
/// user's ends that slide where it shows; the rows keep moving.
@MainActor
final class ListScrollView: NSScrollView, MotionStack {
	/// Every change's slide of the document, as the layer adds them. A slide
	/// goes once it has run, not as its change ends: a change ends before
	/// its motion has (`InboxTransition`), and the offset shows as drawn.
	private var tracks: [MotionTrack] = []
	var now: () -> TimeInterval = { CACurrentMediaTime() }

	override init(frame: NSRect) {
		super.init(frame: frame)
		drawsBackground = false
		hasVerticalScroller = true
		autohidesScrollers = true
		scrollerStyle = .overlay
		contentView = FlippedClipView()
		NotificationCenter.default.addObserver(
			self, selector: #selector(liveScrollStarted(_:)), name: NSScrollView.willStartLiveScrollNotification, object: self,
		)
	}

	@available(*, unavailable)
	required init?(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	/// The offset the clip view is at: the target of every change so far.
	var origin: CGFloat {
		contentView.bounds.minY
	}

	/// The offset that shows at `time`, mid-slide or not.
	func shownOrigin(at time: TimeInterval) -> CGFloat {
		origin - tracks.reduce(0) { $0 + $1.offset(at: time) }
	}

	/// Scrolls to `target` at once and, as change `motion`, slides the
	/// document from `previous`, the offset before the change, which a
	/// resize of the document on the way may have clamped; without a motion,
	/// every slide ends. The document must already be tall enough for the
	/// clip view at `target`.
	func scroll(from previous: CGFloat, to target: CGFloat, as motion: MotionChange?) {
		contentView.scroll(to: NSPoint(x: 0, y: target))
		reflectScrolledClipView(contentView)
		guard let motion else {
			settle()
			return
		}
		let delta = origin - previous
		guard delta != 0, let layer = documentView?.layer else { return }
		let track = MotionTrack(change: motion.change, start: motion.time, duration: motion.duration, delta: delta)
		layer.addMotion(track, keyPath: "position.y", key: "motion.scroll.\(motion.change)")
		tracks.removeAll { $0.end <= motion.time }
		tracks.append(track)
	}

	func complete(_: Int) {
		let time = now()
		tracks.removeAll { $0.end <= time }
	}

	func settle() {
		tracks.removeAll()
		documentView?.layer?.removeMotions(keyedFrom: "motion.scroll.")
	}

	override func scrollWheel(with event: NSEvent) {
		stopSlidingWhereItShows()
		super.scrollWheel(with: event)
	}

	/// A drag of the scroller.
	@objc private func liveScrollStarted(_: Notification) {
		stopSlidingWhereItShows()
	}

	/// The user scrolls from what they see: the clip view takes the offset
	/// as it shows, the document grown to hold it if it must be; the last
	/// completion brings both back within the target.
	private func stopSlidingWhereItShows() {
		let time = now()
		guard tracks.contains(where: { $0.end > time }) else { return }
		let shown = max(shownOrigin(at: time), 0)
		CATransaction.begin()
		CATransaction.setDisableActions(true)
		settle()
		if let documentView, documentView.frame.height < shown + contentView.bounds.height {
			documentView.setFrameSize(NSSize(width: documentView.frame.width, height: shown + contentView.bounds.height))
		}
		contentView.scroll(to: NSPoint(x: 0, y: shown))
		reflectScrolledClipView(contentView)
		CATransaction.commit()
	}
}

/// A clip view whose document hangs from the top.
private final class FlippedClipView: NSClipView {
	override var isFlipped: Bool { true }
}
