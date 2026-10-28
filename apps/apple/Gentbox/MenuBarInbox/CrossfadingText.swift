import AppKit
import QuartzCore

/// A label whose text cross-fades in place: the text it showed fades out
/// in a ghost, a copy of the label left where the label stood, as the new
/// text fades in. Ghosts still fading wait for their fade to run, so a text
/// replaced twice in quick succession never jumps.
@MainActor
final class CrossfadingText {
	private struct Ghost {
		let view: NSTextField
		var fade: ContentMotion?
	}

	let label: NSTextField
	private let makeLabel: @MainActor () -> NSTextField
	private var labelFade: ContentMotion?
	private var ghosts: [Ghost] = []

	init(_ makeLabel: @escaping @MainActor () -> NSTextField) {
		self.makeLabel = makeLabel
		label = makeLabel()
		label.wantsLayer = true
	}

	/// Puts the label in `view`, with a ghost made ahead under it.
	func add(to view: NSView) {
		view.addSubview(label)
		ghosts.append(Ghost(view: makeGhost(), fade: nil))
	}

	/// Shows `text`: cross-fading from the text shown at `time`, at once
	/// without a time.
	func show(_ text: String, crossfadingAt time: TimeInterval?) {
		let shownText = label.stringValue
		label.stringValue = text
		guard let time else {
			end()
			return
		}
		guard !shownText.isEmpty, shownText != text else { return }
		let index = idleGhostIndex(at: time)
		guard ghosts.indices.contains(index) else { return }
		let ghost = ghosts[index].view
		ghost.stringValue = shownText
		ghost.frame = label.frame
		let shownOpacity = labelFade?.opacity(at: time) ?? Float(label.alphaValue)
		ghosts[index].fade = Self.fade(ghost, from: shownOpacity, to: 0, at: time)
		labelFade = Self.fade(label, from: 0, to: 1, at: time)
	}

	/// Every fade dropped: the label shows its text, the ghosts nothing.
	func end() {
		for index in ghosts.indices {
			ghosts[index].view.layer?.removeMotions(keyedFrom: "motion.")
			ghosts[index].view.alphaValue = 0
			ghosts[index].fade = nil
		}
		label.layer?.removeMotions(keyedFrom: "motion.")
		label.alphaValue = 1
		labelFade = nil
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

	private func makeGhost() -> NSTextField {
		let ghost = makeLabel()
		ghost.isSelectable = false
		ghost.wantsLayer = true
		ghost.alphaValue = 0
		ghost.setAccessibilityElement(false)
		label.superview?.addSubview(ghost, positioned: .below, relativeTo: label)
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
