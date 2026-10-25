import QuartzCore
import XCTest

/// The motions an item takes as it comes and goes.
@MainActor
final class LayerMotionTests: XCTestCase {
	/// An AppKit view's layer: anchored at its corner.
	private func cornerAnchoredLayer() -> CALayer {
		let layer = CALayer()
		layer.anchorPoint = .zero
		layer.bounds = CGRect(x: 0, y: 0, width: 100, height: 40)
		return layer
	}

	func testAZoomScalesAboutTheCentreWhereverTheAnchorIs() throws {
		let layer = cornerAnchoredLayer()
		LayerMotion.zoom(layer, from: 0.9, to: 1, duration: 0.3)
		let zoom = try XCTUnwrap(layer.animation(forKey: "motion.transform") as? CABasicAnimation)
		let from = try XCTUnwrap((zoom.fromValue as? NSValue)?.caTransform3DValue)
		let centre = CGAffineTransform(a: from.m11, b: from.m12, c: from.m21, d: from.m22, tx: from.m41, ty: from.m42)
		XCTAssertEqual(CGPoint(x: 50, y: 20).applying(centre).x, 50, accuracy: 0.001)
		XCTAssertEqual(CGPoint(x: 50, y: 20).applying(centre).y, 20, accuracy: 0.001)
		XCTAssertEqual(CGPoint.zero.applying(centre).x, 5, accuracy: 0.001)
		XCTAssertTrue(CATransform3DIsIdentity(layer.transform), "the model stays unscaled")
		XCTAssertTrue(zoom.isRemovedOnCompletion, "growing to full size leaves nothing behind")
	}

	func testAZoomThatEndsSmallerHoldsUntilEnded() throws {
		let layer = cornerAnchoredLayer()
		LayerMotion.zoom(layer, from: 1, to: 0.96, duration: 0.3)
		let zoom = try XCTUnwrap(layer.animation(forKey: "motion.transform"))
		XCTAssertFalse(zoom.isRemovedOnCompletion)
		XCTAssertEqual(zoom.fillMode, .forwards)
		LayerMotion.endZoom(layer)
		XCTAssertNil(layer.animation(forKey: "motion.transform"))
	}

	func testAnArrivingItemOpensItsPlaceFromTheTop() throws {
		let layer = cornerAnchoredLayer()
		LayerMotion.grow(layer, fromHeight: 0, duration: 0.3)
		let grow = try XCTUnwrap(layer.animation(forKey: "motion.bounds") as? CABasicAnimation)
		XCTAssertEqual((grow.fromValue as? NSValue)?.rectValue, NSRect(x: 0, y: 0, width: 100, height: 0))
		XCTAssertEqual((grow.toValue as? NSValue)?.rectValue, NSRect(x: 0, y: 0, width: 100, height: 40))
	}

	func testSlowMotionLengthensEveryMotion() {
		let real = Tokens.motionDuration
		Tokens.motionScale = 4
		defer { Tokens.motionScale = 1 }
		XCTAssertEqual(Tokens.motionDuration, real * 4, accuracy: 0.0001)
		XCTAssertEqual(Tokens.spawnDuration, 0.18 * 4, accuracy: 0.0001)
	}
}
