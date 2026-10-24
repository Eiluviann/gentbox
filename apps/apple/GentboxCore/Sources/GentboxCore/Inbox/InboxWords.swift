import Foundation

/// The words and symbols the inbox draws, kept out of the views so the
/// tests can check them. Single-sentence UI copy has no trailing period.
public enum InboxWords {
	/// "now", "4 min ago", "2 h ago", "3 d ago".
	public static func timeAgo(_ date: Date, now: Date = Date()) -> String {
		let seconds = max(0, now.timeIntervalSince(date))
		switch seconds {
		case ..<60: return "now"
		case ..<3600: return "\(Int(seconds / 60)) min ago"
		case ..<86400: return "\(Int(seconds / 3600)) h ago"
		default: return "\(Int(seconds / 86400)) d ago"
		}
	}

	/// The second line of a row: conversation · agent · time ago.
	public static func rowContext(for message: InboxMessage, now: Date = Date()) -> [String] {
		[message.conversationTitle, message.agentName, timeAgo(message.receivedAt, now: now)]
			.compactMap { $0 }
			.filter { !$0.isEmpty }
	}

	/// The footer line for muted groups.
	public static func mutedSummary(_ muted: [MutedGroup]) -> String? {
		switch muted.count {
		case 0: return nil
		case 1: return "\(muted[0].title) muted"
		default: return "\(muted.count) topics and threads muted"
		}
	}

	/// The outline SF Symbol for a message type; `checkmark.shield` when a
	/// decision asks for permission (it has Approve, offered now or not).
	public static func typeSymbol(for message: InboxMessage) -> String {
		switch message.messageType {
		case .decisionRequired:
			return message.asksPermission ? "checkmark.shield" : "arrow.triangle.branch"
		case .actionRequired: return "hand.tap"
		case .taskCompleted: return "checkmark.circle"
		case .generic: return "info.circle"
		}
	}

	/// The state icon's accessibility label; `nil` when the agent keeps working.
	public static func waitingLabel(for blocking: Blocking) -> String? {
		switch blocking {
		case .blocking: "Agent waiting"
		case .deferrable: "Paused, can defer"
		case .nonBlocking: nil
		}
	}

	/// The symbols every surface may draw. Each was checked against
	/// `symbol_restrictions.strings`; `SymbolPolicyTests` checks again.
	public static let symbolsInUse = [
		"arrow.triangle.branch", "checkmark.shield", "hand.tap", "checkmark.circle", "info.circle",
		"pause.circle", "figure.walk.motion", "moon.zzz", "bell.slash", "bell", "archivebox",
		"folder", "ellipsis", "gearshape", "tray", "arrow.uturn.backward", "envelope.open",
		"arrow.up.forward.app", "checkmark", "xmark", "arrowshape.turn.up.left", "forward",
		"bubble.left", "power",
	]
}

/// Agent-chosen SF Symbol names (a topic's `style.symbol`) are drawn only
/// when they exist and are not on the system's restricted list, whose
/// symbols may only depict Apple's own products and services.
public struct SymbolPolicy: Sendable {
	/// The system's list of restricted symbols.
	public static let restrictionsPath =
		"/System/Library/CoreServices/CoreGlyphs.bundle/Contents/Resources/symbol_restrictions.strings"

	/// The restricted names; `nil` when the list could not be read, and
	/// then no agent symbol is drawn.
	private let restricted: Set<String>?
	private let exists: @Sendable (String) -> Bool

	/// - Parameters:
	///   - restricted: the restricted names; by default read from the system.
	///   - exists: whether the system has the symbol.
	public init(
		restricted: Set<String>? = SymbolPolicy.systemRestrictions(),
		exists: @escaping @Sendable (String) -> Bool,
	) {
		self.restricted = restricted
		self.exists = exists
	}

	/// `name` when it may be drawn, else `fallback`.
	public func symbol(_ name: String?, fallback: String) -> String {
		guard let name, !name.isEmpty, let restricted, !restricted.contains(name), exists(name) else {
			return fallback
		}
		return name
	}

	/// The restricted names, or `nil` when the list is unreadable.
	public static func systemRestrictions() -> Set<String>? {
		guard let dictionary = NSDictionary(contentsOfFile: restrictionsPath) as? [String: Any] else {
			return nil
		}
		return Set(dictionary.keys)
	}
}
