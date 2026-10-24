import SwiftUI

/// The design's colours and metrics (design/project/an.css). Type colours
/// stay fixed; text and lines follow the system so dark mode reads.
enum Tokens {
	static let decision = Color(red: 0x58 / 255, green: 0x56 / 255, blue: 0xD6 / 255)
	static let action = Color(red: 0x0A / 255, green: 0x82 / 255, blue: 0x70 / 255)
	static let done = Color(red: 0x2E / 255, green: 0x9E / 255, blue: 0x4F / 255)
	static let info = Color(red: 0x8E / 255, green: 0x8E / 255, blue: 0x93 / 255)
	/// Agent waiting, urgent badge.
	static let waiting = Color(red: 0xD7 / 255, green: 0x00 / 255, blue: 0x15 / 255)
	/// Paused · can defer.
	static let paused = Color(red: 0xE8 / 255, green: 0x71 / 255, blue: 0x0A / 255)
	static let secondaryText = Color.secondary
	static let track = Color.primary.opacity(0.05)

	static let popoverWidth: CGFloat = 380
	static let listMaxHeight: CGFloat = 480
	static let rowHorizontalPadding: CGFloat = 14
	static let iconColumn: CGFloat = 16
	static let columnGap: CGFloat = 10
	/// A thread header and its rows indent under the topic's symbol.
	static let threadIndent: CGFloat = 40
}
