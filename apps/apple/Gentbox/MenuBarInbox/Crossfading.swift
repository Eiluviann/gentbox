import AppKit
import QuartzCore

/// A leaf whose fades its owner ends and whose ghosts it makes ahead,
/// whatever the leaf shows.
@MainActor
protocol CrossfadingPart: AnyObject {
	/// Every fade dropped: the leaf shows its model, the ghosts nothing.
	func end()
	/// A ghost made in a quiet turn, so the first cross-fade builds nothing
	/// in its frame.
	func makeGhostAhead()
}

/// A leaf view whose content cross-fades in place on the motion clock: what
/// it showed fades out in a ghost, a copy of the leaf left where the leaf
/// stood, as the new content fades in. Ghosts still fading wait for their
/// fade to run, so content replaced twice in quick succession never jumps.
/// The leaf's own opacity fades on the same clock, from what it shows.
@MainActor
final class Crossfading<Leaf: NSView, Shown: Equatable>: CrossfadingPart {
	private struct Ghost {
		let view: Leaf
		var fade: ContentMotion?
	}

	let leaf: Leaf
	/// What the leaf shows; none before its first `show`.
	private(set) var shown: Shown?
	private let makeLeaf: @MainActor () -> Leaf
	private let write: @MainActor (Leaf, Shown) -> Void
	private let prepareGhost: @MainActor (Leaf) -> Void
	private var leafFade: ContentMotion?
	private var ghosts: [Ghost] = []

	/// `write` puts what is shown in a leaf, the leaf or a ghost at the
	/// leaf's frame; `prepareGhost` sets a ghost apart from the leaf.
	init(
		_ makeLeaf: @escaping @MainActor () -> Leaf,
		write: @escaping @MainActor (Leaf, Shown) -> Void,
		prepareGhost: @escaping @MainActor (Leaf) -> Void = { _ in },
	) {
		self.makeLeaf = makeLeaf
		self.write = write
		self.prepareGhost = prepareGhost
		leaf = makeLeaf()
		leaf.wantsLayer = true
	}

	/// Puts the leaf in `view`, with `ghostsAhead` ghosts made under it.
	func add(to view: NSView, ghostsAhead: Int = 1) {
		view.addSubview(leaf)
		for _ in 0 ..< ghostsAhead {
			ghosts.append(Ghost(view: makeGhost(), fade: nil))
		}
	}

	func makeGhostAhead() {
		guard ghosts.isEmpty, leaf.superview != nil else { return }
		ghosts.append(Ghost(view: makeGhost(), fade: nil))
	}

	/// Shows `newShown`: cross-fading from what the leaf shows at `time`, at
	/// once without a time. The leaf fades in to its opacity.
	func show(_ newShown: Shown, crossfadingAt time: TimeInterval?) {
		let previous = shown
		update(newShown)
		guard let time else {
			end()
			return
		}
		guard let previous, previous != newShown else { return }
		let index = idleGhostIndex(at: time)
		let ghost = ghosts[index].view
		ghost.frame = leaf.frame
		write(ghost, previous)
		ghosts[index].fade = Self.fade(ghost, from: shownOpacity(at: time), to: 0, at: time)
		leafFade = Self.fade(leaf, from: 0, to: Float(leaf.alphaValue), at: time)
	}

	/// Shows `newShown` in the leaf at once, every fade left running: a
	/// change too small to fade, as the minute ticking.
	func update(_ newShown: Shown) {
		guard newShown != shown else { return }
		shown = newShown
		write(leaf, newShown)
	}

	/// Fades the leaf to `opacity` from what it shows at `time`; at once
	/// without a time. A fade under way ends where the leaf's opacity is, so
	/// an opacity it has already keeps it.
	func fade(to opacity: CGFloat, at time: TimeInterval?) {
		guard leaf.alphaValue != opacity else { return }
		guard let time else {
			leaf.layer?.removeAnimation(forKey: ContentMotion.opacityKey)
			leafFade = nil
			leaf.alphaValue = opacity
			return
		}
		leafFade = Self.fade(leaf, from: shownOpacity(at: time), to: Float(opacity), at: time)
	}

	func end() {
		for index in ghosts.indices {
			ghosts[index].view.layer?.removeMotions(keyedFrom: "motion.")
			ghosts[index].view.alphaValue = 0
			ghosts[index].fade = nil
		}
		leaf.layer?.removeMotions(keyedFrom: "motion.")
		leafFade = nil
	}

	private func shownOpacity(at time: TimeInterval) -> Float {
		leafFade?.opacity(at: time) ?? Float(leaf.alphaValue)
	}

	/// A ghost whose fade has run by `time`, made if every one still fades.
	private func idleGhostIndex(at time: TimeInterval) -> Int {
		let idle = ghosts.firstIndex { ghost in
			ghost.fade.map { time >= $0.start + $0.spec.delay + $0.spec.duration } ?? true
		}
		if let idle {
			return idle
		}
		ghosts.append(Ghost(view: makeGhost(), fade: nil))
		return ghosts.count - 1
	}

	private func makeGhost() -> Leaf {
		let ghost = makeLeaf()
		ghost.wantsLayer = true
		ghost.alphaValue = 0
		ghost.setAccessibilityElement(false)
		prepareGhost(ghost)
		leaf.superview?.addSubview(ghost, positioned: .below, relativeTo: leaf)
		return ghost
	}

	private static func fade(_ view: NSView, from fromOpacity: Float, to toOpacity: Float, at time: TimeInterval) -> ContentMotion {
		let spec = MotionSpec.Content(
			fromOpacity: fromOpacity, toOpacity: toOpacity, fromScale: 1, toScale: 1, duration: MotionSpec.crossfadeDuration, delay: 0,
		)
		let fade = ContentMotion(spec: spec, start: time)
		fade.commit(to: view)
		return fade
	}
}

/// A label whose text cross-fades in place.
typealias CrossfadingText = Crossfading<NSTextField, String>

extension Crossfading where Leaf == NSTextField, Shown == String {
	convenience init(_ makeLabel: @escaping @MainActor () -> NSTextField) {
		self.init(makeLabel, write: { $0.stringValue = $1 }, prepareGhost: { $0.isSelectable = false })
	}

	var label: NSTextField {
		leaf
	}
}
