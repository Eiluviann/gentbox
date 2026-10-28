import QuartzCore
import XCTest

/// The render server's clock: a change ends when an invisible sentinel
/// animation, as long as the change, has run on the host's layer tree.
@MainActor
final class LayerMotionClockTests: XCTestCase {
	func testASentinelAsLongAsTheChangeCallsBack() async throws {
		let host = CALayer()
		let clock = LayerMotionClock(on: host)
		let sentinelLayer = try XCTUnwrap(host.sublayers?.first, "the clock's layer sits in the host")
		XCTAssertEqual(sentinelLayer.bounds, .zero, "of no size")
		let first = expectation(description: "the first change ends")
		let second = expectation(description: "the second change ends")
		clock.whenEnded(at: clock.now + 0.05) { first.fulfill() }
		clock.whenEnded(at: clock.now + 0.1) { second.fulfill() }
		let sentinels = (sentinelLayer.animationKeys() ?? []).compactMap { sentinelLayer.animation(forKey: $0) as? CABasicAnimation }
		XCTAssertEqual(sentinelLayer.animationKeys()?.count, 2, "one key per change")
		let durations = sentinels.map(\.duration).sorted()
		XCTAssertEqual(durations.count, 2)
		XCTAssertEqual(durations.first ?? 0, 0.05, accuracy: 1e-3, "as long as its change")
		XCTAssertEqual(durations.last ?? 0, 0.1, accuracy: 1e-3)
		for sentinel in sentinels {
			XCTAssertTrue(sentinel.isAdditive)
			XCTAssertEqual(sentinel.fromValue as? Double, 0, "shows nothing")
			XCTAssertEqual(sentinel.toValue as? Double, 0)
		}
		await fulfillment(of: [first, second], timeout: 2)
	}

	func testAChangeAlreadyOverStillCallsBack() async {
		let clock = LayerMotionClock(on: CALayer())
		let ended = expectation(description: "called back")
		clock.whenEnded(at: clock.now - 1) { ended.fulfill() }
		await fulfillment(of: [ended], timeout: 2)
	}
}
