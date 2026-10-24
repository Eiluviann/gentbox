import AppKit
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
	/// The inbox's surface: `.popover { background: rgba(255, 255, 255, 0.97) }`
	/// (an.css:193, `--surface` #ffffff at :10). The design has no dark
	/// theme; dark mode takes the system's window background, never a tint.
	static let surface = NSColor(name: "GentboxSurface") { appearance in
		appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
			? .windowBackgroundColor
			: NSColor(white: 1, alpha: 0.97)
	}
	/// The panel's hairline: the 0.5 px ring of `--shadow-window` (an.css:35).
	static let surfaceOutline = NSColor(name: "GentboxSurfaceOutline") { appearance in
		appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
			? NSColor(white: 1, alpha: 0.15)
			: NSColor(white: 0, alpha: 0.2)
	}
	/// The expanded card: `.pp-hero { background: #fff }` (an.css:1147).
	static let card = Color(nsColor: NSColor(name: "GentboxCard") { appearance in
		appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? .windowBackgroundColor : .white
	})
	/// The collapsed rows: `.pp-line { background: #f8f8fa }` (an.css:1148),
	/// a very light grey under the white card.
	static let row = Color(nsColor: NSColor(name: "GentboxRow") { appearance in
		appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
			? .controlBackgroundColor
			: NSColor(red: 0xF8 / 255, green: 0xF8 / 255, blue: 0xFA / 255, alpha: 1)
	})
	/// The rule between items: `--line` #e0e0e0 (an.css:13).
	static let line = Color(nsColor: .separatorColor)
	/// A command in the card: `.pp-cmd { background: var(--track) }` #f2f2f7
	/// (an.css:1149).
	static let codeBox = Color(nsColor: NSColor(name: "GentboxCodeBox") { appearance in
		appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
			? NSColor(white: 1, alpha: 0.08)
			: NSColor(red: 0xF2 / 255, green: 0xF2 / 255, blue: 0xF7 / 255, alpha: 1)
	})
	/// A row under the pointer: `.pp-item.hover` rgba(0, 0, 0, 0.035).
	static let hover = Color.primary.opacity(0.035)
	/// A row picked for a bulk Dismiss: `.row.sel` rgba(0, 122, 255, 0.07).
	static let picked = Color(red: 0, green: 122 / 255, blue: 1).opacity(0.07)
	static let track = Color.primary.opacity(0.05)

	static let popoverWidth: CGFloat = 380
	static let listMaxHeight: CGFloat = 480
	static let rowHorizontalPadding: CGFloat = 14
	static let iconColumn: CGFloat = 16
	static let columnGap: CGFloat = 10
}
