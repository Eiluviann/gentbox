import XCTest

/// A row the list no longer shows fades out, parks under a filter, and goes
/// once its message closes, whether that happens while it fades or after.
final class UnclaimedRowFateTests: XCTestCase {
	func testAShownRowLeaves() {
		XCTAssertEqual(UnclaimedRowFate.of(wasShown: true, isOpen: true, isParked: false), .leave)
		XCTAssertEqual(UnclaimedRowFate.of(wasShown: true, isOpen: false, isParked: false), .leave)
	}

	func testAParkedRowWhoseMessageClosedGoes() {
		XCTAssertEqual(UnclaimedRowFate.of(wasShown: false, isOpen: false, isParked: true), .remove)
	}

	func testAParkedRowStillOpenStays() {
		XCTAssertEqual(UnclaimedRowFate.of(wasShown: false, isOpen: true, isParked: true), .keep)
	}

	/// Taken out ahead of the hub, then closed by its refresh mid-fade: it
	/// fades out first and goes when it has faded.
	func testARowStillFadingOutFinishesItsFade() {
		XCTAssertEqual(UnclaimedRowFate.of(wasShown: false, isOpen: false, isParked: false), .keep)
	}
}
