import Foundation
import Security

/// Where the hub keeps `hub.db`, `hub.lock`, `rules.toml` and the `API`
/// discovery folder, by how the build is signed (architecture §3).
public struct StateFolder: Equatable, Sendable {
	/// How the folder was chosen.
	public enum Source: Equatable, Sendable {
		/// The App Group container the signature claims: the paid team's
		/// `group.com.eiluviann.gentbox`, or a Personal Team's
		/// `<TEAM>.com.eiluviann.gentbox`.
		case appGroup(String)
		/// `~/Library/Application Support/Gentbox`: ad hoc or unsigned builds.
		case applicationSupport
		/// `GENTBOX_STATE_DIR`, for the UI tests.
		case override
	}

	/// The folder.
	public let url: URL
	/// How it was chosen.
	public let source: Source

	/// The environment variable that overrides the folder (UI tests).
	public static let overrideVariable = "GENTBOX_STATE_DIR"
	/// With the override, `1` empties the folder before the hub starts, so
	/// each UI test begins with an empty inbox.
	public static let resetVariable = "GENTBOX_STATE_DIR_RESET"
	/// The Info.plist key naming the App Group the build is meant to claim.
	public static let appGroupInfoKey = "GentboxAppGroup"

	public init(url: URL, source: Source) {
		self.url = url
		self.source = source
	}

	/// The folder for this process: the override, else the App Group the
	/// signature actually claims, else Application Support.
	///
	/// The App Group is used only when the running signature claims it,
	/// read through `SecTask`: an unsigned or ad hoc build of a project set
	/// up for a team names a group in Info.plist that it cannot open.
	public static func resolve(
		environment: [String: String] = ProcessInfo.processInfo.environment,
		bundle: Bundle = .main,
	) -> StateFolder {
		if let override = environment[overrideVariable], !override.isEmpty {
			let url = URL(fileURLWithPath: override, isDirectory: true)
			if environment[resetVariable] == "1" {
				try? FileManager.default.removeItem(at: url)
			}
			return StateFolder(url: url, source: .override)
		}
		if let group = bundle.object(forInfoDictionaryKey: appGroupInfoKey) as? String,
			!group.isEmpty,
			claimedAppGroups().contains(group),
			let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
		{
			return StateFolder(url: container, source: .appGroup(group))
		}
		return StateFolder(url: applicationSupportFolder(), source: .applicationSupport)
	}

	/// `~/Library/Application Support/Gentbox` in the real home folder, not
	/// the sandbox container's: the ad hoc entitlements open exactly this
	/// path, and the CLI looks for it there.
	static func applicationSupportFolder() -> URL {
		realHomeFolder()
			.appendingPathComponent("Library/Application Support/Gentbox", isDirectory: true)
	}

	/// The user's home folder from the password database; inside the
	/// sandbox `NSHomeDirectory()` is the container.
	public static func realHomeFolder() -> URL {
		if let entry = getpwuid(getuid()), let directory = entry.pointee.pw_dir {
			return URL(fileURLWithPath: String(cString: directory), isDirectory: true)
		}
		return FileManager.default.homeDirectoryForCurrentUser
	}

	/// The App Groups the running signature claims.
	static func claimedAppGroups() -> [String] {
		guard let task = SecTaskCreateFromSelf(nil),
			let value = SecTaskCopyValueForEntitlement(
				task,
				"com.apple.security.application-groups" as CFString,
				nil,
			)
		else {
			return []
		}
		return (value as? [String]) ?? []
	}
}
