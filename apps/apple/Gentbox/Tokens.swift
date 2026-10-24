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
	/// Deny keeps the system red.
	static let danger = Color(red: 0xD7 / 255, green: 0x00 / 255, blue: 0x15 / 255)
	/// The Claude mark's own colour.
	static let claude = Color(red: 0xD9 / 255, green: 0x77 / 255, blue: 0x57 / 255)
	static let controlBackground = Color(nsColor: .controlBackgroundColor)
	/// `--control-line` (#d1d1d6) in light mode.
	static let controlLine = Color(nsColor: .separatorColor)
	/// The faded dots between breadcrumb parts (#c7c7cc).
	static let dot = Color(red: 0xC7 / 255, green: 0xC7 / 255, blue: 0xCC / 255)
	/// The list under the card sits on light grey; the card stays white.
	static let listBackground = Color.primary.opacity(0.025)
	static let track = Color.primary.opacity(0.05)

	static let popoverWidth: CGFloat = 380
	static let listMaxHeight: CGFloat = 480
	static let rowHorizontalPadding: CGFloat = 14
	static let iconColumn: CGFloat = 16
	static let columnGap: CGFloat = 10
}
