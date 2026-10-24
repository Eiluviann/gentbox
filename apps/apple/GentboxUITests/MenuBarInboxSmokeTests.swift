import Foundation
import XCTest

/// The smoke test of PR 3: the app launches as the hub, a message posted with
/// `curl` appears in the menu-bar inbox, rows carry topic › thread (a simple
/// conversation lands in General, which the breadcrumb leaves out), topics
/// and threads mute from the item menu, a picked row takes the card, and an
/// answer reaches the waiting sender.
///
/// Each test runs the app on `~/Library/Application Support/Gentbox/UITests`,
/// which the ad hoc entitlements open and the app empties at launch, so it
/// never touches your real inbox. Xcode sandboxes the test runner with
/// read-only access to the disk: the test reads the discovery files and
/// leaves every write to the app.
final class MenuBarInboxSmokeTests: XCTestCase {
	private var app: XCUIApplication!
	private var stateFolder: URL!

	override func setUpWithError() throws {
		continueAfterFailure = false
		stateFolder = Self.realHome()
			.appendingPathComponent("Library/Application Support/Gentbox/UITests", isDirectory: true)
		app = XCUIApplication()
		app.launchEnvironment["GENTBOX_STATE_DIR"] = stateFolder.path
		app.launchEnvironment["GENTBOX_STATE_DIR_RESET"] = "1"
		app.launchEnvironment["GENTBOX_SNAPSHOT_PATH"] = stateFolder.appendingPathComponent("snapshot.png").path
		app.launchArguments += ["-GentboxSkipLoginItem", "YES"]
		if Self.isMenuBarAutoHidden {
			app.launchArguments += ["-GentboxOpenInboxAtLaunch", "YES"]
		}
		let launchedAt = Date()
		app.launch()
		try waitForTheHub(startedAfter: launchedAt)
	}

	override func tearDownWithError() throws {
		app?.terminate()
	}

	func testACurlPostedMessageAppearsWithItsConversationAsTheBreadcrumb() throws {
		try post([
			"type": "task_completed",
			"title": "Ingest tests pass",
			"conversation": ["id": "session-1", "title": "Refactor the store"],
		])
		openInbox()
		XCTAssertTrue(element("message:Ingest tests pass").waitForExistence(timeout: 10))
		// A simple conversation lands in General, which the breadcrumb leaves out.
		// A row is one element: title, breadcrumb and agent in one label.
		let context = text(of: element("message:Ingest tests pass"))
		XCTAssertTrue(context.contains("Refactor the store"), context)
		XCTAssertFalse(context.contains("General"), context)
		XCTAssertTrue(context.contains("Claude Code"), "the agent mark is labelled: \(context)")
	}

	func testRowsCarryTopicAndThreadNewestFirst() throws {
		try post([
			"type": "task_completed", "title": "Store migrations run on start",
			"topic": ["key": "gentbox", "title": "Gentbox"],
			"thread": ["key": "phase-3", "title": "Phase 3"],
		])
		try post([
			"type": "task_completed", "title": "Staging certificates rotated",
			"source": ["agent": "codex", "displayName": "Codex"], "conversation": ["id": "cx-2"],
			"topic": ["key": "infra", "title": "Infrastructure"],
			"thread": ["key": "certs", "title": "Rotate staging certificates"],
		])
		try post([
			"type": "generic", "title": "Release notes drafted",
			"topic": ["key": "gentbox"],
			"thread": ["key": "v1-1", "title": "v1.1"],
		])
		openInbox()
		XCTAssertTrue(element("message:Release notes drafted").waitForExistence(timeout: 10))
		XCTAssertTrue(text(of: element("message:Store migrations run on start")).contains("Gentbox › Phase 3"))
		XCTAssertTrue(text(of: element("message:Release notes drafted")).contains("Gentbox › v1.1"))
		let certificates = text(of: element("message:Staging certificates rotated"))
		XCTAssertTrue(certificates.contains("Infrastructure › Rotate staging certificates"), certificates)
		XCTAssertTrue(certificates.contains("Codex"), certificates)
		assertOrder([
			"message:Release notes drafted",
			"message:Staging certificates rotated",
			"message:Store migrations run on start",
		])

		// Close leads to the thread's open items instead of closing it.
		element("message:Store migrations run on start").rightClick()
		app.menuItems["Close thread"].click()
		let banner = element("inbox.filter")
		XCTAssertTrue(banner.waitForExistence(timeout: 5))
		XCTAssertEqual(text(of: banner), "1 item still open in Phase 3")
		XCTAssertTrue(element("message:Release notes drafted").waitForNonExistence(timeout: 5))
		element("inbox.show-all").click()
		XCTAssertTrue(element("message:Release notes drafted").waitForExistence(timeout: 5))
	}

	func testMutingATopicAndAThreadFromTheItemMenu() throws {
		try post([
			"type": "task_completed", "title": "Tokio upgraded",
			"topic": ["key": "deps", "title": "Dependency upgrades"],
			"thread": ["key": "tokio", "title": "Tokio 1.55"],
		])
		try post([
			"type": "task_completed", "title": "Search filters persist in the URL",
			"conversation": ["id": "session-2"],
			"topic": ["key": "web", "title": "Web"],
			"thread": ["key": "search", "title": "Search filters"],
		])
		openInbox()
		let tokio = element("message:Tokio upgraded")
		XCTAssertTrue(tokio.waitForExistence(timeout: 10))
		tokio.rightClick()
		app.menuItems["Mute topic Dependency upgrades"].click()
		// The row itself says so, for VoiceOver.
		assertValue("Muted", of: tokio)
		// And the foot names what is muted.
		let summary = element("inbox.muted-summary")
		XCTAssertTrue(summary.waitForExistence(timeout: 5))
		XCTAssertEqual(text(of: summary), "Dependency upgrades muted")

		let search = element("message:Search filters persist in the URL")
		search.rightClick()
		app.menuItems["Mute thread Search filters"].click()
		assertValue("Muted", of: search)
		let both = NSPredicate(format: "value == '2 topics and threads muted' OR label == '2 topics and threads muted'")
		let updated = XCTNSPredicateExpectation(predicate: both, object: summary)
		XCTAssertEqual(XCTWaiter.wait(for: [updated], timeout: 5), .completed, text(of: summary))
	}

	func testTheNextUpCardOffersAnswersWhileTheSenderWaits() throws {
		try post([
			"type": "task_completed", "title": "Search filters persist in the URL; 18 tests pass",
			"source": ["agent": "codex", "displayName": "Codex"], "conversation": ["id": "cx-1", "title": "web"],
			"topic": ["key": "web", "title": "Web"], "thread": ["key": "web-139", "title": "WEB-139 search filters"],
		])
		try post([
			"type": "decision_required", "blocking": "deferrable",
			"title": "Which cluster gets the new certificates first?",
			"source": ["agent": "codex", "displayName": "Codex"], "conversation": ["id": "cx-2", "title": "infra"],
			"topic": ["key": "infra", "title": "Infrastructure"],
			"thread": ["key": "certs", "title": "Rotate staging certificates"],
		])
		try post([
			"type": "task_completed", "urgent": true, "title": "Ingest auth accepts rotated tokens; 12 tests pass",
			"conversation": ["id": "cc-3", "title": "ingest"],
			"topic": ["key": "gentbox", "title": "Gentbox"], "thread": ["key": "phase-3", "title": "Phase 3"],
		])
		try post([
			"type": "generic", "title": "Nightly build log is ready",
			"conversation": ["id": "cc-4", "title": "Refactor the store"],
		])
		let prompt = try post([
			"type": "decision_required", "blocking": "blocking", "urgent": true, "awaitsResponse": true,
			"title": "Allow Bash to run the ingest tests?",
			"body": "cargo test -p gentbox-ingest -- --nocapture",
			"conversation": ["id": "cc-1", "title": "ingest"],
			"topic": ["key": "gentbox", "title": "Gentbox"], "thread": ["key": "phase-3", "title": "Phase 3"],
			"actions": [
				["id": "allow", "label": "Approve", "kind": "approve"],
				["id": "deny", "label": "Deny", "kind": "deny"],
				["id": "reply", "label": "Reply", "kind": "reply"],
			],
		])
		// The hook's long-poll: the answer buttons show only while it waits.
		let waiter = try longPoll(prompt)
		defer { waiter.terminate() }

		openInbox()
		let card = element("card:Allow Bash to run the ingest tests?")
		// With the inbox open from launch, the first blocked-on message to
		// arrive was pinned, and a later prompt never takes its place: expand
		// the prompt by its row.
		if !card.waitForExistence(timeout: 3) {
			element("message:Allow Bash to run the ingest tests?").click()
		}
		XCTAssertTrue(card.waitForExistence(timeout: 10))
		XCTAssertTrue(element("action:Approve").waitForExistence(timeout: 5))
		XCTAssertTrue(element("action:Deny").exists)
		attachSnapshot(named: "Menu-bar inbox")

		// The accordion: clicking a collapsed row expands it in place and
		// collapses the card that was open; its buttons are the live ones.
		element("message:Search filters persist in the URL; 18 tests pass").click()
		let picked = element("card:Search filters persist in the URL; 18 tests pass")
		XCTAssertTrue(picked.waitForExistence(timeout: 5))
		XCTAssertTrue(card.waitForNonExistence(timeout: 5))
		XCTAssertTrue(element("message:Allow Bash to run the ingest tests?").exists)
		XCTAssertTrue(element("action:Dismiss").isHittable)
		attachSnapshot(named: "Expanded row")
		// Its header (here, the breadcrumb in it) collapses it, leaving
		// nothing expanded.
		element("context:Search filters persist in the URL; 18 tests pass").click()
		XCTAssertTrue(picked.waitForNonExistence(timeout: 5))
		attachSnapshot(named: "All collapsed")
		element("message:Allow Bash to run the ingest tests?").click()
		XCTAssertTrue(card.waitForExistence(timeout: 5))
		XCTAssertTrue(element("action:Approve").isHittable)

		element("action:Approve").click()
		XCTAssertTrue(card.waitForNonExistence(timeout: 5))
		waiter.waitUntilExit()
		let answer = String(decoding: (waiter.standardOutput as? Pipe)?.fileHandleForReading.readDataToEndOfFile() ?? Data(), as: UTF8.self)
		XCTAssertTrue(answer.contains("\"answered\""), answer)
		XCTAssertTrue(answer.contains("\"allow\""), answer)
		attachSnapshot(named: "After Approve")
	}

	func testADeferredActionStaysAnswerableWithDone() throws {
		let action = try post([
			"type": "action_required", "blocking": "deferrable", "awaitsResponse": true,
			"title": "Sign in to the payments sandbox",
			"steps": [["text": "Sign in to the payments sandbox", "detail": "Use the team account"]],
		])
		let waiter = try longPoll(action)
		defer { waiter.terminate() }
		openInbox()
		let card = element("card:Sign in to the payments sandbox")
		XCTAssertTrue(card.waitForExistence(timeout: 10))
		element("action:Defer").click()
		waiter.waitUntilExit()
		let deferred = String(decoding: (waiter.standardOutput as? Pipe)?.fileHandleForReading.readDataToEndOfFile() ?? Data(), as: UTF8.self)
		XCTAssertTrue(deferred.contains("\"deferred\""), deferred)

		// Deferred, it no longer blocks the agent, but it stays open, and its
		// pinned card keeps Done in reach to finish it.
		XCTAssertTrue(card.exists)
		let done = element("action:Done")
		XCTAssertTrue(done.waitForExistence(timeout: 5))
		XCTAssertFalse(element("action:Defer").exists)
		done.click()
		XCTAssertTrue(card.waitForNonExistence(timeout: 5))
		XCTAssertFalse(element("message:Sign in to the payments sandbox").exists)
	}

	/// The panel's height moves with the content, frame by frame: its bottom
	/// edge stays on the content's bottom (header at the top, foot at the
	/// bottom) while the accordion expands and collapses, a refresh lands
	/// mid-motion, and items leave and arrive. XCUITest waits for animations
	/// to end before it reads anything, so the app records its own frames
	/// (`AnimationTrace`, debug builds) and this test reads that record.
	///
	/// The trace samples between display frames, every 20 to 40 ms, so
	/// mid-motion the two may read apart by as much as the panel moves
	/// between two samples, never more: a panel that lagged or led the
	/// content would drift further. Settled, they agree.
	func testThePanelsHeightMovesWithTheContent() throws {
		let frames = try traceFrames()
		let steps = Dictionary(grouping: frames) { $0.step }
		XCTAssertEqual(
			Set(steps.keys),
			["expand-row", "collapse", "expand-another", "expand-then-refresh", "removal", "arrival"],
		)
		for (step, frames) in steps {
			XCTAssertGreaterThan(frames.count, 5, step)
			if step != "expand-then-refresh" {
				// That one swaps two cards of one height.
				XCTAssertGreaterThan(Set(frames.map(\.panelHeight)).count, 3, "the panel did not move in \(step)")
			}
			// A capture taken while SwiftUI redraws can miss the marker (-99);
			// CI's screen recording showed the marker on the panel's edge at
			// such a frame. One per step is a capture glitch, more is a gap.
			let unreadable = frames.filter { $0.gap <= -99 }
			XCTAssertLessThanOrEqual(unreadable.count, 1, "\(step): the content's bottom is cut off at \(unreadable.map(\.ms)) ms")
			for (previous, frame) in zip(frames, frames.dropFirst()) where frame.gap > -99 {
				let sampleMotion = Double(abs(frame.panelHeight - previous.panelHeight))
				XCTAssertLessThanOrEqual(
					abs(frame.gap), sampleMotion + 1.5,
					"\(step) at \(frame.ms) ms: the content's bottom is \(frame.gap) pt from the panel's, which moved \(sampleMotion) pt since the last sample",
				)
			}
			for frame in frames.suffix(3) {
				XCTAssertTrue((0...1).contains(frame.gap), "\(step) settled \(frame.gap) pt from the panel's bottom")
			}
		}
	}

	/// The panel never moves: from opening to closing its top edge and left
	/// edge stay where they were set, and only the bottom edge moves.
	func testThePanelsTopEdgeNeverMoves() throws {
		let frames = try traceFrames()
		XCTAssertGreaterThan(frames.count, 30)
		XCTAssertEqual(Set(frames.map(\.panelTop)).count, 1, "the top edge moved: \(Set(frames.map(\.panelTop)))")
		XCTAssertEqual(Set(frames.map(\.panelX)).count, 1, "the panel slid: \(Set(frames.map(\.panelX)))")
	}

	/// The card's keys work in the panel, which never activates the app: ↩
	/// approves, ⌘⌫ denies, R opens the reply field, where typing and ↩ send.
	func testTheCardsKeysAnswerInThePanel() throws {
		let prompts = [
			("Allow editing a.ts?", "approve"),
			("Allow editing b.ts?", "deny"),
			("Allow editing c.ts?", "reply"),
		]
		var waiters: [String: Process] = [:]
		for (index, (title, _)) in prompts.enumerated() {
			let id = try post([
				"type": "decision_required", "blocking": "blocking", "awaitsResponse": true, "title": title,
				"conversation": ["id": "keys-\(index)"],
				"actions": [
					["id": "allow", "label": "Approve", "kind": "approve"],
					["id": "deny", "label": "Deny", "kind": "deny"],
					["id": "reply", "label": "Reply", "kind": "reply"],
				],
			])
			waiters[title] = try longPoll(id)
		}
		defer { waiters.values.forEach { $0.terminate() } }
		openInbox()

		for _ in prompts {
			// The expanded card is the pinned next-up prompt; act on it by key.
			let expanded = try XCTUnwrap(
				prompts.first { element("card:\($0.0)").waitForExistence(timeout: 5) },
				"no card expanded",
			)
			let card = element("card:\(expanded.0)")
			// A click in the card's empty corner gives the panel the keyboard.
			card.coordinate(withNormalizedOffset: CGVector(dx: 0.97, dy: 0.9)).click()
			switch expanded.1 {
			case "approve":
				app.typeKey(.return, modifierFlags: [])
			case "deny":
				app.typeKey(.delete, modifierFlags: .command)
			default:
				app.typeKey("r", modifierFlags: [])
				let field = element("reply-field")
				XCTAssertTrue(field.waitForExistence(timeout: 5))
				field.click()
				field.typeText("Use the staging cluster")
				app.typeKey(.return, modifierFlags: [])
			}
			let waiter = try XCTUnwrap(waiters[expanded.0])
			waiter.waitUntilExit()
			let answer = String(decoding: (waiter.standardOutput as? Pipe)?.fileHandleForReading.readDataToEndOfFile() ?? Data(), as: UTF8.self)
			XCTAssertTrue(answer.contains("\"answered\""), "\(expanded.0): \(answer)")
			switch expanded.1 {
			case "approve": XCTAssertTrue(answer.contains("\"allow\""), answer)
			case "deny": XCTAssertTrue(answer.contains("\"deny\""), answer)
			default: XCTAssertTrue(answer.contains("Use the staging cluster"), answer)
			}
			XCTAssertTrue(card.waitForNonExistence(timeout: 5))
		}
	}

	// MARK: Helpers

	private struct TraceFrame {
		let step: String
		let ms: Int
		let panelX: Int
		let panelTop: Int
		let panelHeight: Int
		let gap: Double
	}

	/// Launches the app with `AnimationTrace` over five items, posts one more
	/// when the trace waits for an arrival, and reads its record.
	private func traceFrames() throws -> [TraceFrame] {
		app.terminate()
		let trace = stateFolder.appendingPathComponent("trace", isDirectory: true)
		app.launchEnvironment["GENTBOX_TRACE_DIR"] = trace.path
		if !app.launchArguments.contains("-GentboxOpenInboxAtLaunch") {
			app.launchArguments += ["-GentboxOpenInboxAtLaunch", "YES"]
		}
		let launchedAt = Date()
		app.launch()
		try waitForTheHub(startedAfter: launchedAt)
		for index in 1...6 {
			if index == 6 {
				let awaiting = trace.appendingPathComponent("awaiting-arrival")
				let deadline = Date().addingTimeInterval(40)
				while !FileManager.default.fileExists(atPath: awaiting.path), Date() < deadline {
					Thread.sleep(forTimeInterval: 0.2)
				}
			}
			try post([
				"type": "task_completed", "title": "Completed task number \(index)",
				"body": "line one\nline two\nline three",
				"conversation": ["id": "session-\(index)"],
				"topic": ["key": "web", "title": "Web"],
				"thread": ["key": "t\(index)", "title": "Thread \(index)"],
			])
		}
		let csv = trace.appendingPathComponent("frames.csv")
		let deadline = Date().addingTimeInterval(30)
		while !FileManager.default.fileExists(atPath: csv.path), Date() < deadline {
			Thread.sleep(forTimeInterval: 0.5)
		}
		return try String(contentsOf: csv, encoding: .utf8)
			.split(separator: "\n").dropFirst()
			.map { $0.split(separator: ",").map(String.init) }
			.compactMap { fields in
				guard fields.count == 8, let ms = Int(fields[2]), let x = Int(fields[3]), let top = Int(fields[4]),
					let height = Int(fields[5]), let gap = Double(fields[7])
				else { return nil }
				return TraceFrame(step: fields[1], ms: ms, panelX: x, panelTop: top, panelHeight: height, gap: gap)
			}
	}

	private func assertValue(_ value: String, of element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
		let isSet = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", value), object: element)
		XCTAssertEqual(
			XCTWaiter.wait(for: [isSet], timeout: 5), .completed,
			"\(element.identifier) has value \(String(describing: element.value))", file: file, line: line,
		)
	}

	/// Waits for `API/port` and `API/token`: the hub is up and serving.
	/// A `port` left by the previous test's app (killed, so it never removed
	/// it) must not count: only one written after this launch does.
	private func waitForTheHub(startedAfter launchedAt: Date) throws {
		let api = stateFolder.appendingPathComponent("API")
		let deadline = Date().addingTimeInterval(20)
		while Date() < deadline {
			let port = api.appendingPathComponent("port")
			let token = api.appendingPathComponent("token")
			let written = (try? FileManager.default.attributesOfItem(atPath: port.path))?[.modificationDate] as? Date
			if let written, written >= launchedAt.addingTimeInterval(-1),
				FileManager.default.fileExists(atPath: token.path)
			{
				return
			}
			Thread.sleep(forTimeInterval: 0.2)
		}
		XCTFail("the hub did not start in \(stateFolder.path)")
	}

	/// `curl -X POST /v1/messages` over loopback with the sender token, as an
	/// agent would.
	@discardableResult
	private func post(_ fields: [String: Any]) throws -> String {
		let (port, token) = try discovery()
		var message: [String: Any] = [
			"version": 1,
			"source": ["agent": "claude-code", "displayName": "Claude Code"],
			"conversation": ["id": "session-1", "cwd": "/Users/me/Developer/gentbox"],
		]
		message.merge(fields) { _, new in new }
		let body = try JSONSerialization.data(withJSONObject: message)

		let curl = Process()
		curl.executableURL = URL(fileURLWithPath: "/usr/bin/curl")
		curl.arguments = [
			"--silent", "--show-error", "--write-out", "\n%{http_code}",
			// A throwaway token of the test's own state folder; a real client
			// keeps it out of argv.
			"--header", "Authorization: Bearer \(token)",
			"--header", "Content-Type: application/json",
			"--data-binary", "@-",
			"http://127.0.0.1:\(port)/v1/messages",
		]
		let input = Pipe()
		let output = Pipe()
		curl.standardInput = input
		curl.standardOutput = output
		try curl.run()
		input.fileHandleForWriting.write(body)
		try input.fileHandleForWriting.close()
		curl.waitUntilExit()
		let printed = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
		let lines = printed.split(separator: "\n", omittingEmptySubsequences: false)
		let status = lines.last.map(String.init) ?? ""
		XCTAssertEqual(status, "201", "POST /v1/messages answered \(printed)")
		let response = lines.dropLast().joined(separator: "\n")
		let json = try JSONSerialization.jsonObject(with: Data(response.utf8)) as? [String: Any]
		return json?["id"] as? String ?? ""
	}

	/// `GET /v1/messages/{id}/response?wait=55` in the background, as the
	/// Claude Code hook waits; its output is the answer.
	private func longPoll(_ id: String) throws -> Process {
		let (port, token) = try discovery()
		let curl = Process()
		curl.executableURL = URL(fileURLWithPath: "/usr/bin/curl")
		curl.arguments = [
			"--silent", "--show-error", "--header", "Authorization: Bearer \(token)",
			"http://127.0.0.1:\(port)/v1/messages/\(id)/response?wait=55",
		]
		curl.standardOutput = Pipe()
		try curl.run()
		// Let the poll reach the hub before the inbox is read.
		Thread.sleep(forTimeInterval: 0.5)
		return curl
	}

	private func discovery() throws -> (port: String, token: String) {
		let api = stateFolder.appendingPathComponent("API")
		let read = { (name: String) in
			try String(contentsOf: api.appendingPathComponent(name), encoding: .utf8)
				.trimmingCharacters(in: .whitespacesAndNewlines)
		}
		return (try read("port"), try read("token"))
	}

	/// Keeps the app's own snapshot of the inbox (open-at-launch mode only) in
	/// the result bundle (`xcrun xcresulttool export attachments`), for the
	/// PR and design review.
	private func attachSnapshot(named name: String) {
		guard Self.isMenuBarAutoHidden else { return }
		// The app writes it 0.4 s after the inbox changes.
		Thread.sleep(forTimeInterval: 1.5)
		let url = stateFolder.appendingPathComponent("snapshot.png")
		guard let data = try? Data(contentsOf: url) else { return }
		let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.png")
		attachment.name = name
		attachment.lifetime = .keepAlways
		add(attachment)
	}

	/// The home folder from the password database: inside the runner's
	/// sandbox `NSHomeDirectory()` is its container.
	private static func realHome() -> URL {
		if let entry = getpwuid(getuid()), let directory = entry.pointee.pw_dir {
			return URL(fileURLWithPath: String(cString: directory), isDirectory: true)
		}
		return URL(fileURLWithPath: NSHomeDirectory())
	}

	/// Opens the inbox panel from the status item. With the menu bar hidden
	/// automatically the item cannot be clicked, so the app opened the panel
	/// at launch (`-GentboxOpenInboxAtLaunch`); the item is still checked.
	private func openInbox() {
		let item = statusItem()
		XCTAssertTrue(item.waitForExistence(timeout: 10), "no status item")
		XCTAssertEqual(item.label, "Gentbox")
		if !Self.isMenuBarAutoHidden {
			item.click()
		}
	}

	/// System Settings › Control Centre › "Automatically hide and show the
	/// menu bar".
	private static var isMenuBarAutoHidden: Bool {
		UserDefaults.standard.bool(forKey: "_HIHideMenuBar")
	}

	private func statusItem() -> XCUIElement {
		let inMenuBar = app.menuBars.statusItems["gentbox.status-item"]
		return inMenuBar.exists ? inMenuBar : app.statusItems["gentbox.status-item"]
	}

	/// A static text's string: macOS puts it in the value, not the label.
	private func text(of element: XCUIElement) -> String {
		element.label.isEmpty ? (element.value as? String ?? "") : element.label
	}

	private func element(_ identifier: String) -> XCUIElement {
		app.descendants(matching: .any)[identifier].firstMatch
	}

	/// The elements exist and sit top to bottom in this order.
	private func assertOrder(_ identifiers: [String], file: StaticString = #filePath, line: UInt = #line) {
		let tops = identifiers.map { identifier -> CGFloat in
			let found = element(identifier)
			XCTAssertTrue(found.waitForExistence(timeout: 5), "\(identifier) is missing", file: file, line: line)
			return found.frame.minY
		}
		XCTAssertEqual(tops, tops.sorted(), "out of order: \(identifiers)", file: file, line: line)
	}
}
