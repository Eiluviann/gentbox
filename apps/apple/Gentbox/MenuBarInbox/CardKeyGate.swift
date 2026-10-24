import QuartzCore

/// When a card takes keys. A card that has just shown, whether expanded,
/// handed over from one just answered or replaced in place by a superseding
/// message, takes keys only once its motion is over; and a card that has
/// just been answered takes no more until the next message shows. So a
/// second ↩ meant for what was there never answers what is not yet seen.
struct CardKeyGate {
	private var opensAt: CFTimeInterval = 0
	private var messageId: String?

	/// The card shows now: keys wait for its motion.
	mutating func cardShown(at now: CFTimeInterval) {
		opensAt = now + Tokens.motionDuration
	}

	/// The card shows `messageId`. Another message than before, on a card
	/// that shows, waits for the motion again.
	mutating func show(messageId: String, isShown: Bool, at now: CFTimeInterval) {
		defer { self.messageId = messageId }
		guard messageId != self.messageId else { return }
		opensAt = isShown && self.messageId != nil ? now + Tokens.motionDuration : min(opensAt, now)
	}

	/// The card has been answered: no more keys until another message shows.
	mutating func answered() {
		opensAt = .infinity
	}

	func isOpen(at now: CFTimeInterval) -> Bool {
		now >= opensAt
	}
}
