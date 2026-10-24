import XCTest

/// A card takes keys only once its motion is over, and none after its
/// answer until the next message shows.
@MainActor
final class CardKeyGateTests: XCTestCase {
	private let motion = Tokens.motionDuration

	func testACardThatShowsWaitsForItsMotion() {
		var gate = CardKeyGate()
		gate.show(messageId: "a", isShown: false, at: 10)
		XCTAssertTrue(gate.isOpen(at: 10))
		gate.cardShown(at: 10)
		XCTAssertFalse(gate.isOpen(at: 10 + motion / 2))
		XCTAssertTrue(gate.isOpen(at: 10 + motion))
	}

	func testASupersedingMessageOnAShownCardWaitsAgain() {
		var gate = CardKeyGate()
		gate.show(messageId: "a", isShown: false, at: 0)
		gate.cardShown(at: 0)
		gate.show(messageId: "a", isShown: true, at: 5)
		XCTAssertTrue(gate.isOpen(at: 5), "the same message keeps its keys")
		gate.show(messageId: "a2", isShown: true, at: 5)
		XCTAssertFalse(gate.isOpen(at: 5.01), "↩ 10 ms after the swap must not answer it unseen")
		XCTAssertTrue(gate.isOpen(at: 5 + motion))
	}

	func testAnAnsweredCardTakesNoKeysUntilTheNextMessage() {
		var gate = CardKeyGate()
		gate.show(messageId: "a", isShown: false, at: 0)
		gate.cardShown(at: 0)
		gate.answered()
		XCTAssertFalse(gate.isOpen(at: 100))
		gate.show(messageId: "a", isShown: true, at: 100)
		XCTAssertFalse(gate.isOpen(at: 100), "the answered message again")
		gate.show(messageId: "b", isShown: true, at: 100)
		XCTAssertFalse(gate.isOpen(at: 100))
		XCTAssertTrue(gate.isOpen(at: 100 + motion))
	}

	func testANewMessageOnAHiddenCardIsOpenOnceShown() {
		var gate = CardKeyGate()
		gate.show(messageId: "a", isShown: false, at: 0)
		gate.answered()
		gate.show(messageId: "b", isShown: false, at: 50)
		XCTAssertTrue(gate.isOpen(at: 50), "a collapsed card's gate waits for it to show")
		gate.cardShown(at: 60)
		XCTAssertFalse(gate.isOpen(at: 60))
	}
}
