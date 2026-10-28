import AppKit
import QuartzCore
import XCTest

/// A label's text cross-fading in place, as the render server draws it,
/// sampled at 120 Hz off the fades on the label's and its ghosts' layers:
/// each text's opacity moves on the curve, even when the text is replaced
/// again mid-fade, and never jumps.
@MainActor
final class CrossfadingTextTests: XCTestCase {
	private struct Change {
		/// After the playback's start; none for a text shown at once.
		let time: TimeInterval?
		let text: String
	}

	private typealias Sample = [String: CGFloat]

	private static let frameRate = 120.0
	/// The curve's steepest slope (`MotionTimelineTests`).
	private static let steepestSlope = 4.0451
	private static let tolerance: CGFloat = 1e-6
	/// Far from 0, which Core Animation reads as "now".
	private static let origin: TimeInterval = 1000

	private var duration: TimeInterval {
		MotionSpec.crossfadeDuration
	}

	func testANewTextFadesInAsTheOldFadesOut() {
		let (text, container) = makeText(showing: "Waiting")
		text.show("Approved", crossfadingAt: Self.origin)
		let samples = play(container, through: duration)
		assertNoJumps(samples)
		for sample in samples {
			XCTAssertEqual((sample["Waiting"] ?? 0) + (sample["Approved"] ?? 0), 1, accuracy: Self.tolerance, "one text's worth of ink")
		}
		XCTAssertEqual(samples.first?["Waiting"], 1)
		XCTAssertEqual(samples.first?["Approved"], 0)
		XCTAssertEqual(samples.last?["Waiting"] ?? 0, 0, accuracy: Self.tolerance)
		XCTAssertEqual(samples.last?["Approved"] ?? 0, 1, accuracy: Self.tolerance)
		XCTAssertEqual(text.label.stringValue, "Approved")
	}

	func testATextReplacedMidFadeFadesOutFromWhereItShows() {
		let (text, container) = makeText(showing: "Waiting")
		let samples = play(text, in: container, [
			Change(time: 0, text: "Approved"),
			Change(time: 6 / Self.frameRate, text: "Sent"),
		])
		assertNoJumps(samples)
		XCTAssertEqual(fields(in: container).count, 3, "a second ghost for the text still fading in")
		let replaced = samples[safe: 6]?["Approved"] ?? 0
		XCTAssertEqual(replaced, CGFloat(MotionCurve.standard.value(at: 6 / Self.frameRate / duration)), accuracy: Self.tolerance)
		XCTAssertEqual(samples[safe: 7]?["Sent"] ?? 0, CGFloat(MotionCurve.standard.value(at: 1 / Self.frameRate / duration)), accuracy: 1e-4)
		XCTAssertEqual(samples.last?["Waiting"] ?? 1, 0, accuracy: Self.tolerance)
		XCTAssertEqual(samples.last?["Approved"] ?? 1, 0, accuracy: Self.tolerance)
		XCTAssertEqual(samples.last?["Sent"] ?? 0, 1, accuracy: Self.tolerance)
	}

	func testAGhostWhoseFadeHasRunIsReused() {
		let (text, container) = makeText(showing: "Waiting")
		let samples = play(text, in: container, [
			Change(time: 0, text: "Approved"),
			Change(time: duration + 1 / Self.frameRate, text: "Sent"),
		])
		assertNoJumps(samples)
		XCTAssertEqual(fields(in: container).count, 2, "the label and the one ghost made ahead")
		XCTAssertEqual(samples.last?["Approved"] ?? 1, 0, accuracy: Self.tolerance)
		XCTAssertEqual(samples.last?["Sent"] ?? 0, 1, accuracy: Self.tolerance)
	}

	func testWithoutATimeTheTextShowsAtOnce() {
		let (text, container) = makeText(showing: "Waiting")
		text.show("Approved", crossfadingAt: Self.origin)
		text.show("Sent", crossfadingAt: nil)
		let sample = shownOpacities(in: container, at: Self.origin + 0.05)
		XCTAssertEqual(sample, ["Waiting": 0, "Sent": 1])
		XCTAssertTrue(fields(in: container).allSatisfy { motionKeys($0).isEmpty }, "every fade dropped")
	}

	func testTheSameTextShowsOnWithoutAFade() {
		let (text, container) = makeText(showing: "Waiting")
		text.show("Waiting", crossfadingAt: Self.origin)
		XCTAssertEqual(shownOpacities(in: container, at: Self.origin), ["Waiting": 1])
		XCTAssertTrue(fields(in: container).allSatisfy { motionKeys($0).isEmpty })
	}

	// MARK: Helpers

	private func makeText(showing shownText: String) -> (CrossfadingText, NSView) {
		let container = FlippedView(frame: NSRect(x: 0, y: 0, width: 300, height: 40))
		container.wantsLayer = true
		let text = CrossfadingText { Label.wrapping(font: .systemFont(ofSize: 13)) }
		text.add(to: container)
		text.label.frame = NSRect(x: 12, y: 8, width: 276, height: 20)
		text.show(shownText, crossfadingAt: nil)
		return (text, container)
	}

	/// Each text's opacity at every frame from the playback's start through
	/// `end`, its fades already committed.
	private func play(_ container: NSView, through end: TimeInterval) -> [Sample] {
		frames(through: end).map { shownOpacities(in: container, at: Self.origin + $0) }
	}

	/// Each text's opacity at every frame, the changes landing as their
	/// frame comes, through the last one's fade.
	private func play(_ text: CrossfadingText, in container: NSView, _ changes: [Change]) -> [Sample] {
		var pending = changes[...]
		return frames(through: (changes.last?.time ?? 0) + duration).map { time in
			while let next = pending.first, (next.time ?? 0) <= time {
				text.show(next.text, crossfadingAt: next.time.map { Self.origin + $0 })
				pending = pending.dropFirst()
			}
			return shownOpacities(in: container, at: Self.origin + time)
		}
	}

	/// Each text's opacity at `time`, the most any field showing it shows.
	private func shownOpacities(in container: NSView, at time: TimeInterval) -> Sample {
		fields(in: container).filter { !$0.stringValue.isEmpty }.reduce(into: [:]) { shown, field in
			shown[field.stringValue] = max(shown[field.stringValue] ?? 0, shownOpacity(of: field, at: time))
		}
	}

	/// A field's opacity as the render server draws it: held at the fade's
	/// start before it begins, the model once it ends.
	private func shownOpacity(of field: NSTextField, at time: TimeInterval) -> CGFloat {
		guard let fade = field.layer?.animation(forKey: ContentMotion.opacityKey) as? CABasicAnimation,
		      let from = (fade.fromValue as? NSNumber)?.doubleValue,
		      let to = (fade.toValue as? NSNumber)?.doubleValue
		else { return field.alphaValue }
		let progress = (time - fade.beginTime) / fade.duration
		if progress < 0 { return CGFloat(from) }
		guard progress < 1 else { return field.alphaValue }
		return CGFloat(from + (to - from) * MotionCurve.standard.value(at: progress))
	}

	private func assertNoJumps(_ samples: [Sample], file: StaticString = #filePath, line: UInt = #line) {
		let maxStep = Self.steepestSlope / (Self.frameRate * duration) + Self.tolerance
		for (index, (earlier, later)) in zip(samples, samples.dropFirst()).enumerated() {
			for text in Set(earlier.keys).union(later.keys) {
				let step = abs((later[text] ?? 0) - (earlier[text] ?? 0))
				XCTAssertLessThanOrEqual(step, maxStep, "\(text) jumps at frame \(index + 1)", file: file, line: line)
			}
		}
	}

	private func fields(in container: NSView) -> [NSTextField] {
		container.subviews.compactMap { $0 as? NSTextField }
	}

	private func motionKeys(_ view: NSView) -> [String] {
		(view.layer?.animationKeys() ?? []).filter { $0.hasPrefix("motion.") }
	}

	/// Every frame's time at 120 Hz from 0 through `end`.
	private func frames(through end: TimeInterval) -> [TimeInterval] {
		(0 ... Int((end * Self.frameRate).rounded(.up))).map { Double($0) / Self.frameRate }
	}
}

private extension Array {
	subscript(safe index: Int) -> Element? {
		indices.contains(index) ? self[index] : nil
	}
}
