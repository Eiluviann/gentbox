import CoreGraphics
import XCTest

/// The one table of what content does as its slot changes: the fades stay
/// under Reduce Motion, the zooms go.
@MainActor
final class MotionSpecTests: XCTestCase {
	func testArrivalsFadeInAndZoomUp() {
		for kind in [MotionSpec.Kind.insert, .open] {
			XCTAssertEqual(
				MotionSpec.content(kind, reducesMotion: false),
				MotionSpec.Content(fromOpacity: 0, toOpacity: 1, fromScale: 0.96, toScale: 1, duration: 0.18, delay: 0),
			)
		}
	}

	func testDeparturesFadeOutAndZoomDownLessForThePanel() {
		XCTAssertEqual(
			MotionSpec.content(.remove, reducesMotion: false),
			MotionSpec.Content(fromOpacity: 1, toOpacity: 0, fromScale: 1, toScale: 0.96, duration: 0.12, delay: 0),
		)
		XCTAssertEqual(
			MotionSpec.content(.close, reducesMotion: false),
			MotionSpec.Content(fromOpacity: 1, toOpacity: 0, fromScale: 1, toScale: 0.98, duration: 0.12, delay: 0),
		)
	}

	/// The body comes in once the card has made room, and goes before it
	/// closes over it.
	func testACardsBodyFadesInLateAndOutQuickly() {
		XCTAssertEqual(
			MotionSpec.content(.expand, reducesMotion: false),
			MotionSpec.Content(fromOpacity: 0, toOpacity: 1, fromScale: 1, toScale: 1, duration: 0.15, delay: 0.15),
		)
		XCTAssertEqual(
			MotionSpec.content(.collapse, reducesMotion: false),
			MotionSpec.Content(fromOpacity: 1, toOpacity: 0, fromScale: 1, toScale: 1, duration: 0.08, delay: 0),
		)
	}

	func testReduceMotionKeepsTheFadesAndDropsTheZooms() {
		for kind in [MotionSpec.Kind.insert, .remove, .expand, .collapse, .open, .close] {
			let full = MotionSpec.content(kind, reducesMotion: false)
			let reduced = MotionSpec.content(kind, reducesMotion: true)
			XCTAssertEqual(reduced.fromScale, 1, "\(kind)")
			XCTAssertEqual(reduced.toScale, 1, "\(kind)")
			XCTAssertEqual(reduced.fromOpacity, full.fromOpacity, "\(kind)")
			XCTAssertEqual(reduced.toOpacity, full.toOpacity, "\(kind)")
			XCTAssertEqual(reduced.duration, full.duration, "\(kind)")
			XCTAssertEqual(reduced.delay, full.delay, "\(kind)")
		}
	}

	func testEverythingSlowsWithSlowMotion() {
		Tokens.motionScale = 4
		defer { Tokens.motionScale = 1 }
		XCTAssertEqual(MotionSpec.content(.insert, reducesMotion: false).duration, 0.72, accuracy: 1e-9)
		XCTAssertEqual(MotionSpec.content(.expand, reducesMotion: false).delay, 0.6, accuracy: 1e-9)
		XCTAssertEqual(MotionSpec.crossfadeDuration, 1.2, accuracy: 1e-9)
	}

	func testTheTokensCurveIsTheModelsCurve() {
		var tokens: [Float] = [0, 0]
		var model: [Float] = [0, 0]
		for index in 0 ..< 4 {
			Tokens.motionCurve.getControlPoint(at: index, values: &tokens)
			MotionCurve.standard.timingFunction.getControlPoint(at: index, values: &model)
			XCTAssertEqual(tokens, model)
		}
	}

	func testSlowMotionLengthensEveryMotion() {
		let real = Tokens.motionDuration
		Tokens.motionScale = 4
		defer { Tokens.motionScale = 1 }
		XCTAssertEqual(Tokens.motionDuration, real * 4, accuracy: 0.0001)
		XCTAssertEqual(Tokens.spawnDuration, 0.18 * 4, accuracy: 0.0001)
	}
}
