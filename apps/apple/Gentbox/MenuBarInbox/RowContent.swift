import GentboxCore

/// Everything a row draws: a row changes only when this does.
struct RowContent: Equatable {
	let message: InboxMessage
	let isExpanded: Bool
	let isPicked: Bool
	/// Rows are being picked for a bulk Dismiss: each one that can be picked
	/// shows a checkbox in place of its type icon.
	let isSelecting: Bool
	/// The first item draws no rule above it.
	let isFirst: Bool
	/// The reply box is open on this message. Its text lives in the field
	/// while it is typed, so a keystroke changes no row.
	let isReplying: Bool
	let moveTargets: [TopicChoice]

	/// Only whether rows are being picked, or this one is, changed.
	func differsOnlyInPicking(from other: RowContent) -> Bool {
		self != other && message == other.message && isExpanded == other.isExpanded && isFirst == other.isFirst
			&& isReplying == other.isReplying && moveTargets == other.moveTargets
	}
}
