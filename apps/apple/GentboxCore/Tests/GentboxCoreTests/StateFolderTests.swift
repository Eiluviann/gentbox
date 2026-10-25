import Foundation
import Testing
@testable import GentboxCore

/// `GENTBOX_STATE_DIR` and its reset are for test builds only: a build that
/// does not allow the override neither moves its state nor deletes a folder
/// named in the environment.
struct StateFolderTests {
	@Test func withoutTheOverrideAllowedTheEnvironmentIsIgnored() throws {
		let folder = try makeFolder()
		defer { try? FileManager.default.removeItem(at: folder) }
		let environment = [StateFolder.overrideVariable: folder.path, StateFolder.resetVariable: "1"]
		let resolved = StateFolder.resolve(environment: environment, allowsOverride: false)
		#expect(resolved.source != .override)
		#expect(resolved.url.standardizedFileURL != folder.standardizedFileURL)
		#expect(FileManager.default.fileExists(atPath: folder.appendingPathComponent("hub.db").path), "the folder was emptied")
	}

	@Test func withTheOverrideAllowedTheTestsGetTheirOwnEmptyFolder() throws {
		let folder = try makeFolder()
		defer { try? FileManager.default.removeItem(at: folder) }
		let environment = [StateFolder.overrideVariable: folder.path, StateFolder.resetVariable: "1"]
		let resolved = StateFolder.resolve(environment: environment, allowsOverride: true)
		#expect(resolved.source == .override)
		#expect(resolved.url.standardizedFileURL == folder.standardizedFileURL)
		#expect(!FileManager.default.fileExists(atPath: folder.appendingPathComponent("hub.db").path))
	}

	private func makeFolder() throws -> URL {
		let folder = FileManager.default.temporaryDirectory.appendingPathComponent("gentbox-state-\(UUID().uuidString)", isDirectory: true)
		try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
		try Data().write(to: folder.appendingPathComponent("hub.db"))
		return folder
	}
}
