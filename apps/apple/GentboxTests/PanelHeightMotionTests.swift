import SwiftUI
import XCTest

/// The panel's height eases on the content's curve, and a change that
/// arrives mid-motion adds on top, as SwiftUI combines them.
final class PanelHeightMotionTests: XCTestCase {
	func testItEasesFromTheOldHeightToTheNewOnTheMotionsCurve() {
		var motion = PanelHeightMotion(height: 300, duration: 0.25, curve: .easeInOut)
		motion.retarget(to: 400, at: 10)
		XCTAssertEqual(motion.height(at: 10), 300)
		XCTAssertEqual(motion.height(at: 10.125), 350, accuracy: 0.01)
		XCTAssertEqual(motion.height(at: 10.05), 300 + 100 * UnitCurve.easeInOut.value(at: 0.2), accuracy: 0.01)
		XCTAssertTrue(motion.isMoving(at: 10.2))
		XCTAssertEqual(motion.height(at: 10.25), 400)
		XCTAssertFalse(motion.isMoving(at: 10.25))
	}

	func testAChangeMidMotionAddsItsOwnDifference() {
		var motion = PanelHeightMotion(height: 300, duration: 0.25, curve: .linear)
		motion.retarget(to: 400, at: 0)
		motion.retarget(to: 350, at: 0.125)
		// First change halfway (+50), second just started (-0).
		XCTAssertEqual(motion.height(at: 0.125), 350, accuracy: 0.01)
		// First done (+100), second halfway (-25).
		XCTAssertEqual(motion.height(at: 0.25), 375, accuracy: 0.01)
		XCTAssertEqual(motion.height(at: 0.375), 350, accuracy: 0.01)
		XCTAssertEqual(motion.target, 350)
	}

	func testTheSameHeightStartsNothingAndResetJumps() {
		var motion = PanelHeightMotion(height: 300)
		motion.retarget(to: 300, at: 0)
		XCTAssertFalse(motion.isMoving(at: 0))
		motion.retarget(to: 500, at: 0)
		motion.reset(to: 200)
		XCTAssertEqual(motion.height(at: 0.01), 200)
		XCTAssertFalse(motion.isMoving(at: 0.01))
	}
}
