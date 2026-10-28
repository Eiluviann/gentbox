import AppKit
import XCTest

/// A view that keeps its own stack of slots, as a card's body and the inbox
/// do: once its owner lets it go, the view and its stack are freed, through
/// a change still moving.
@MainActor
final class SlotStackOwnershipTests: XCTestCase {
	private final class StackOwner: FlippedView {
		lazy var parts = SlotStack<String>(in: self, width: 100)
	}

	private static let origin: TimeInterval = 1000

	func testAViewThatKeepsItsStackIsFreedOnceLetGo() {
		weak var freedOwner: StackOwner?
		weak var freedStack: SlotStack<String>?
		let transition = InboxTransition(clock: ManualClock(at: Self.origin))
		autoreleasepool {
			let owner = StackOwner(frame: NSRect(x: 0, y: 0, width: 100, height: 100))
			let slot = SlotView()
			owner.parts.settle([SlotStack.Entry(id: "a", view: slot, height: 20)])
			transition.add(owner.parts)
			let moved = [SlotStack.Entry(id: "a", view: slot, height: 40), SlotStack.Entry(id: "b", view: SlotView(), height: 10)]
			_ = transition.run(for: Tokens.motionDuration) { change, time in
				owner.parts.apply(moved, change: change, at: time, duration: Tokens.motionDuration, reducesMotion: false)
			}
			freedOwner = owner
			freedStack = owner.parts
		}
		XCTAssertNil(freedOwner, "the view is freed")
		XCTAssertNil(freedStack, "its stack with it")
	}
}
