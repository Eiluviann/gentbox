import Foundation
import XCTest

/// The three entitlement sets (docs/signing.md): the reduced ones are
/// subsets of the full one, only the paid team's claims the restricted
/// notification capabilities, and none holds a team id (the repository is
/// public; the team lives in the gitignored Signing.local.xcconfig).
final class EntitlementVariantsTests: XCTestCase {
	private static let timeSensitive = "com.apple.developer.usernotifications.time-sensitive"
	private static let communication = "com.apple.developer.usernotifications.communication"

	func testReducedSetsAreSubsetsOfTheFullSet() throws {
		let full = try entitlements("Full")
		for variant in ["PersonalTeam", "AdHoc"] {
			let reduced = try entitlements(variant)
			for (key, value) in reduced where key != "com.apple.security.temporary-exception.files.home-relative-path.read-write" {
				XCTAssertEqual(full[key] as? NSObject, value as? NSObject, "\(variant) claims \(key) differently")
			}
		}
	}

	func testOnlyThePaidTeamClaimsTimeSensitiveAndCommunicationNotifications() throws {
		let full = try entitlements("Full")
		XCTAssertEqual(full[Self.timeSensitive] as? Bool, true)
		XCTAssertEqual(full[Self.communication] as? Bool, true)
		for variant in ["PersonalTeam", "AdHoc"] {
			let reduced = try entitlements(variant)
			XCTAssertNil(reduced[Self.timeSensitive], variant)
			XCTAssertNil(reduced[Self.communication], variant)
		}
	}

	func testEveryBuildIsSandboxedAndServesTheLocalAPI() throws {
		for variant in ["Full", "PersonalTeam", "AdHoc"] {
			let set = try entitlements(variant)
			XCTAssertEqual(set["com.apple.security.app-sandbox"] as? Bool, true, variant)
			XCTAssertEqual(set["com.apple.security.network.server"] as? Bool, true, variant)
		}
	}

	func testTheAppGroupComesFromTheBuildSettingNotALiteral() throws {
		for variant in ["Full", "PersonalTeam"] {
			let groups = try entitlements(variant)["com.apple.security.application-groups"] as? [String]
			XCTAssertEqual(groups, ["$(GENTBOX_APP_GROUP)"], variant)
		}
	}

	private func entitlements(_ name: String) throws -> [String: Any] {
		let bundle = Bundle(for: Self.self)
		let url = try XCTUnwrap(bundle.url(forResource: name, withExtension: "entitlements"), "\(name).entitlements")
		let data = try Data(contentsOf: url)
		let plist = try PropertyListSerialization.propertyList(from: data, format: nil)
		return try XCTUnwrap(plist as? [String: Any])
	}
}
