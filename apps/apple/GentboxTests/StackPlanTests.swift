import CoreGraphics
import XCTest

/// A change of a stack, diffed by id: one merged order, a frame before and
/// after for every slot, and the fewest movers.
final class StackPlanTests: XCTestCase {
	private typealias Plan = StackPlan<String>

	func testALeavingSlotStaysBetweenItsNeighbours() {
		let plan = Plan(before: slots(["a", "b", "c"]), after: slots(["a", "c"]))
		XCTAssertEqual(plan.order, ["a", "b", "c"])
		XCTAssertEqual(plan.removed, ["b"])
		XCTAssertEqual(plan.after["b"], Plan.Frame(top: 60, height: 0), "it closes where it was")
		XCTAssertEqual(plan.after["c"], Plan.Frame(top: 60, height: 60))
	}

	func testLeavingSlotsKeepTheirOrderAtTheTopAndInRuns() {
		let plan = Plan(before: slots(["a", "b", "c", "d", "e"]), after: slots(["c", "new"]))
		XCTAssertEqual(plan.order, ["a", "b", "c", "d", "e", "new"])
		XCTAssertEqual(plan.removed, ["a", "b", "d", "e"])
		XCTAssertEqual(plan.inserted, ["new"])
		XCTAssertTrue(plan.movers.isEmpty)
	}

	/// A slot still leaving sits at no height in the stack the previous
	/// change aimed at: it stays placed, and comes back as a survivor.
	func testASlotStillLeavingStaysPlacedAndComesBackInPlace() {
		let leaving = [Plan.Slot(id: "a", height: 60), Plan.Slot(id: "b", height: 0), Plan.Slot(id: "c", height: 60)]
		XCTAssertEqual(Plan(before: leaving, after: slots(["a", "c"])).order, ["a", "b", "c"])
		let back = Plan(before: leaving, after: slots(["a", "b", "c"]))
		XCTAssertEqual(back.before["b"], Plan.Frame(top: 60, height: 0))
		XCTAssertEqual(back.after["b"], Plan.Frame(top: 60, height: 60))
		XCTAssertTrue(back.inserted.isEmpty)
		XCTAssertTrue(back.movers.isEmpty)
	}

	/// An inserted slot opens at the bottom of the slot above it, where that
	/// slot was before the change.
	func testAnInsertedSlotOpensUnderTheSlotAbove() {
		let plan = Plan(
			before: [Plan.Slot(id: "a", height: 200), Plan.Slot(id: "b", height: 60)],
			after: [Plan.Slot(id: "new", height: 60), Plan.Slot(id: "a", height: 60), Plan.Slot(id: "x", height: 60), Plan.Slot(id: "b", height: 60)],
		)
		XCTAssertEqual(plan.before["new"], Plan.Frame(top: 0, height: 0))
		XCTAssertEqual(plan.before["x"], Plan.Frame(top: 200, height: 0), "under a as it was, open")
		XCTAssertEqual(plan.after["x"], Plan.Frame(top: 120, height: 60))
	}

	/// The row that loses its agent drops to the bottom: it alone moves, the
	/// rest keep their order.
	func testTheFewestSlotsMove() {
		let plan = Plan(before: slots(["d", "a", "b", "c"]), after: slots(["a", "b", "c", "d"]))
		XCTAssertEqual(plan.movers, ["d"])
		XCTAssertEqual(plan.order, ["a", "b", "c", "d"])
		XCTAssertEqual(plan.before["d"], Plan.Frame(top: 0, height: 60))
		XCTAssertEqual(plan.after["d"], Plan.Frame(top: 180, height: 60))
	}

	/// Of two slots trading places, the smaller moves over the open card.
	func testOfTwoSwappedSlotsTheSmallerMoves() {
		let plan = Plan(
			before: [Plan.Slot(id: "open", height: 200), Plan.Slot(id: "row", height: 60)],
			after: [Plan.Slot(id: "row", height: 60), Plan.Slot(id: "open", height: 200)],
		)
		XCTAssertEqual(plan.movers, ["row"])
	}

	/// A slot removed beside a mover closes where it was rather than ride
	/// across the stack with it.
	func testARemovedSlotNeverRidesWithAMover() {
		let plan = Plan(before: slots(["m", "r", "a", "b"]), after: slots(["a", "b", "m"]))
		XCTAssertEqual(plan.movers, ["m"])
		XCTAssertEqual(plan.order, ["r", "a", "b", "m"])
		XCTAssertEqual(plan.after["r"], Plan.Frame(top: 0, height: 0))
	}

	private func slots(_ ids: [String], height: CGFloat = 60) -> [Plan.Slot] {
		ids.map { Plan.Slot(id: $0, height: height) }
	}
}
