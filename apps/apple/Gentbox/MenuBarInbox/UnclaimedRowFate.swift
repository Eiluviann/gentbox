/// What becomes of a row whose message the list no longer shows.
enum UnclaimedRowFate: Equatable {
	/// It showed: it fades out where it was, then parks or goes, by whether
	/// its message is still open then.
	case leave
	/// It is parked and its message has closed: it goes now.
	case remove
	/// It is still fading out (it decides when it has faded), or parked with
	/// its message still open.
	case keep

	static func of(wasShown: Bool, isOpen: Bool, isParked: Bool) -> Self {
		if wasShown {
			return .leave
		}
		return isParked && !isOpen ? .remove : .keep
	}
}
