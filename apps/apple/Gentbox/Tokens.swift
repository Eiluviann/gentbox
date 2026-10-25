import AppKit
import QuartzCore

/// The design's colours and metrics (design/project/an.css). Type colours
/// stay fixed; text and lines follow the system so dark mode reads.
enum Tokens {
	static let decision = NSColor(srgbRed: 0x58 / 255, green: 0x56 / 255, blue: 0xD6 / 255, alpha: 1)
	static let action = NSColor(srgbRed: 0x0A / 255, green: 0x82 / 255, blue: 0x70 / 255, alpha: 1)
	static let done = NSColor(srgbRed: 0x2E / 255, green: 0x9E / 255, blue: 0x4F / 255, alpha: 1)
	static let info = NSColor(srgbRed: 0x8E / 255, green: 0x8E / 255, blue: 0x93 / 255, alpha: 1)
	/// Agent waiting, urgent badge.
	static let waiting = NSColor(srgbRed: 0xD7 / 255, green: 0x00 / 255, blue: 0x15 / 255, alpha: 1)
	/// Paused · can defer.
	static let paused = NSColor(srgbRed: 0xE8 / 255, green: 0x71 / 255, blue: 0x0A / 255, alpha: 1)
	/// The Claude mark's own colour.
	static let claude = NSColor(srgbRed: 0xD9 / 255, green: 0x77 / 255, blue: 0x57 / 255, alpha: 1)
	/// The faded dots between breadcrumb parts (#c7c7cc).
	static let dot = NSColor(srgbRed: 0xC7 / 255, green: 0xC7 / 255, blue: 0xCC / 255, alpha: 1)
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
	/// The expanded card: white, `.pp-hero { background: #fff }` (an.css:1147).
	/// Dark values are interim system surfaces until the dark design lands.
	static let card = NSColor(name: "GentboxCard") { appearance in
		appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? .windowBackgroundColor : .white
	}
	/// The collapsed rows: white like the card, by the user's instruction,
	/// which overrides the board's light-grey list (`.pp-line { background:
	/// #f8f8fa }`, an.css:1148). Rows are told apart by the hairline rules and
	/// the card by its padding, not by a fill.
	static let row = card
	/// The rule between items: `--line` #e0e0e0 (an.css:13).
	static let line = NSColor.separatorColor
	/// A command in the card: `.pp-cmd { background: var(--track) }` #f2f2f7
	/// (an.css:1149).
	static let codeBox = NSColor(name: "GentboxCodeBox") { appearance in
		appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
			? NSColor(white: 1, alpha: 0.08)
			: NSColor(srgbRed: 0xF2 / 255, green: 0xF2 / 255, blue: 0xF7 / 255, alpha: 1)
	}
	/// A row under the pointer: `.pp-item.hover` rgba(0, 0, 0, 0.035).
	static let hover = NSColor.labelColor.withAlphaComponent(0.035)
	/// A row picked for a bulk Dismiss: `.row.sel` rgba(0, 122, 255, 0.07).
	static let picked = NSColor(srgbRed: 0, green: 122 / 255, blue: 1, alpha: 0.07)
	static let track = NSColor.labelColor.withAlphaComponent(0.05)

	static let popoverWidth: CGFloat = 380
	/// The one motion of every layout change in a Gentbox window: the content
	/// and the window's card move on `motionCurve` over `motionDuration`, as
	/// Core Animation animations committed together, so the card's bottom
	/// edge stays on the content's and no frame of it runs on the main thread.
	@MainActor static var motionDuration: TimeInterval { 0.3 * motionScale }
	/// Eased at both ends, the landing drawn out, so a card visibly settles
	/// into place rather than stopping.
	@MainActor static var motionCurve: CAMediaTimingFunction { CAMediaTimingFunction(controlPoints: 0.2, 0, 0, 1) }
	/// The panel's own appearance and disappearance, quicker than a layout
	/// change, and the card body's exit as its card collapses.
	@MainActor static var spawnDuration: TimeInterval { 0.18 * motionScale }
	@MainActor static var despawnDuration: TimeInterval { 0.12 * motionScale }
	@MainActor static var quickFadeDuration: TimeInterval { 0.08 * motionScale }
	/// How much an item grows into place as it arrives, and shrinks as it
	/// goes.
	static let arrivalScale: CGFloat = 0.96
	#if DEBUG || GENTBOX_HOOKS
	/// Every motion's length is multiplied by this: the debug panel slows
	/// motion down to look at it.
	@MainActor static var motionScale: Double = 1
	#else
	static let motionScale: Double = 1
	#endif
	static let rowHorizontalPadding: CGFloat = 14
	static let iconColumn: CGFloat = 16
	static let columnGap: CGFloat = 10
}
