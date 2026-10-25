import CoreGraphics

/// The list's rows top to bottom as one stack: the listed rows in the list's
/// order, and the rows still closing their place where they were, between
/// the rows they were between. Every row's place comes from this one stack
/// and moves there on the one curve of the change, so a row that leaves
/// moves with its neighbours and no two rows ever cover each other: at each
/// moment of the motion every row is where the stack, part way from before
/// to after, puts it.
enum RowStack {
	/// The rows in the order they show: `listed` as the list has them, and
	/// each of `leaving` placed after the row it followed in `previous`
	/// (first, if none did).
	static func order<Key: Hashable>(listed: [Key], previous: [Key], leaving: Set<Key>) -> [Key] {
		var order = listed
		for (index, key) in previous.enumerated() where leaving.contains(key) && !order.contains(key) {
			let predecessor = previous[..<index].last { order.contains($0) }
			let place = predecessor.flatMap { order.firstIndex(of: $0) }.map { $0 + 1 } ?? 0
			order.insert(key, at: place)
		}
		return order
	}

	/// Each row's top, for rows of `height` stacked in `order` from 0.
	static func tops<Key: Hashable>(of order: [Key], height: (Key) -> CGFloat) -> [Key: CGFloat] {
		var tops: [Key: CGFloat] = [:]
		var y: CGFloat = 0
		for key in order {
			tops[key] = y
			y += height(key)
		}
		return tops
	}
}
