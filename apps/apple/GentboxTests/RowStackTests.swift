import CoreGraphics
import XCTest

/// Rows leaving the list keep their place in the stack while it closes, so
/// no two rows cover each other at any moment of a change.
final class RowStackTests: XCTestCase {
	func testALeavingRowStaysBetweenItsNeighbours() {
		XCTAssertEqual(RowStack.order(listed: ["a", "c"], previous: ["a", "b", "c"], leaving: ["b"]), ["a", "b", "c"])
	}

	func testLeavingRowsKeepTheirOrderAtTheTopAndInRuns() {
		XCTAssertEqual(
			RowStack.order(listed: ["c", "new"], previous: ["a", "b", "c", "d", "e"], leaving: ["a", "b", "d", "e"]),
			["a", "b", "c", "d", "e", "new"],
		)
	}

	func testARowNoLongerLeavingOrNeverShownIsNotPlaced() {
		XCTAssertEqual(RowStack.order(listed: ["a"], previous: ["a", "b"], leaving: []), ["a"])
		XCTAssertEqual(RowStack.order(listed: ["a"], previous: ["a"], leaving: ["z"]), ["a"])
	}

	/// The card above a dismissed one opens as it closes (the hand-over), a
	/// row arrives at the top, and a row below stays: sampled through the
	/// motion, the rows stay one stack, touching and never covering.
	func testNoTwoRowsCoverEachOtherThroughTheMotion() {
		let before: [String: CGFloat] = ["a": 60, "b": 180, "c": 60]
		let beforeOrder = ["a", "b", "c"]
		let afterOrder = RowStack.order(listed: ["new", "a", "c"], previous: beforeOrder, leaving: ["b"])
		XCTAssertEqual(afterOrder, ["new", "a", "b", "c"])
		let after: [String: CGFloat] = ["new": 60, "a": 150, "b": 0, "c": 60]
		let beforeTops = RowStack.tops(of: beforeOrder) { before[$0] ?? 0 }
		let afterTops = RowStack.tops(of: afterOrder) { after[$0] ?? 0 }
		for step in 0...20 {
			// Every row moves on the same curve from the same start: at any
			// moment each is the same fraction of the way.
			let progress = CGFloat(step) / 20
			let frames = afterOrder.map { key in
				// A new row opens from no height at its place.
				let fromTop = beforeTops[key] ?? afterTops[key] ?? 0
				let top = fromTop + ((afterTops[key] ?? 0) - fromTop) * progress
				let height = (before[key] ?? 0) + ((after[key] ?? 0) - (before[key] ?? 0)) * progress
				return (key, top, top + height)
			}
			for (upper, lower) in zip(frames, frames.dropFirst()) {
				XCTAssertEqual(upper.2, lower.1, accuracy: 0.001, "\(upper.0) and \(lower.0) at \(progress)")
			}
		}
	}
}
