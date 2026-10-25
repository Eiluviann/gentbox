import AppKit
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
	/// The trace this test records, attached in tear-down if it failed.
	private var traceInUse: URL?

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

	/// Runs even when a failure stopped the test early.
	override func tearDownWithError() throws {
		app?.terminate()
		if let traceInUse {
			attachTraceOnFailure(from: traceInUse)
		}
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
		// Its header (here, the breadcrumb in it) leaves it expanded: one card
		// is always open.
		element("context:Search filters persist in the URL; 18 tests pass").click()
		XCTAssertFalse(picked.waitForNonExistence(timeout: 1))
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
		// A deferral is no answer: the card keeps its keys, and ↩ is Done.
		app.typeKey(.return, modifierFlags: [])
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
	/// A sample reads the panel's height as just set but the screen as last
	/// composited, so mid-motion the two may read apart by as much as the
	/// panel moves in one display frame, never more: a panel that lagged or
	/// led the content would drift further. Samples come several to a frame,
	/// so the bound is the panel's motion within one 60 Hz frame (CI's
	/// displays) on either side of a sample, or to the next sample when
	/// frames came further apart. Settled, they agree.
	func testThePanelsHeightMovesWithTheContent() throws {
		let frames = try traceFrames(in: "height-moves-with-content")
		let steps = try traceSteps(in: "height-moves-with-content")
		let byStep = Dictionary(grouping: frames) { $0.step }
		XCTAssertEqual(
			Set(byStep.keys),
			[
				"expand-row", "expand-back", "expand-another", "expand-then-refresh", "removal", "arrival", "update",
				"expand-prompt", "waiter", "despawn", "spawn", "reopen-mid-despawn",
			],
		)
		var movingStepsSampled = 0
		for (step, frames) in byStep where !Self.spawnSteps.contains(step) {
			XCTAssertGreaterThan(frames.count, 5, step)
			let missing = frames.filter { $0.gap == nil }.map(\.ms)
			XCTAssertEqual(missing, [], "\(step): no content bottom marker in the frames at these ms")
			for index in frames.indices.dropFirst() {
				guard let gap = frames[index].gap else { continue }
				let sampleMotion = neighbouringMotion(in: frames, at: index)
				XCTAssertLessThanOrEqual(
					abs(gap), sampleMotion + 1.5,
					"\(step) at \(frames[index].ms) ms: the content's bottom is \(gap) pt from the panel's, which moved up to \(sampleMotion) pt within a frame of it",
				)
			}
			for frame in frames.suffix(3) {
				let gapPixels = (frame.gap ?? .infinity) * frame.scale
				XCTAssertLessThanOrEqual(abs(gapPixels), 1, "\(step) settled \(gapPixels) px from the panel's bottom")
			}
			// Eased, not snapped: a step whose height changed shows a frame
			// between before and after, unless the first sample came only
			// after the motion was over (a stalled machine).
			guard let before = steps[step]?.beforeHeight, let after = frames.last?.panelHeight, before != after else {
				continue
			}
			let latency = steps[step]?.latencyMs ?? 0
			if latency >= Int(Self.motionMilliseconds) {
				logMotionCheck("\(step): not checked, the first sample came \(latency) ms after the change")
				continue
			}
			movingStepsSampled += 1
			let between = frames.filter { min(before, after) < $0.panelHeight && $0.panelHeight < max(before, after) }
			XCTAssertFalse(between.isEmpty, "\(step): the panel went from \(before) to \(after) pt with no frame between")
			logMotionCheck("\(step): checked, \(before) → \(after) pt with \(between.count) frames between")
		}
		XCTAssertGreaterThan(movingStepsSampled, 0, "no step that changed height was sampled in motion")
		// Responsiveness: the first frame after a click lands within two
		// display frames, and after it no frame of the motion comes late.
		let frame = Self.displayFrameMilliseconds
		for step in ["expand-row", "expand-back", "expand-another"] {
			let latency = try XCTUnwrap(steps[step]?.latencyMs, step)
			XCTAssertLessThanOrEqual(Double(latency), 2 * frame + 1, "\(step): \(latency) ms to the first frame")
			let moving = frames.filter { $0.step == step && $0.ms <= Int(Self.motionMilliseconds) }
			let late = zip(moving, moving.dropFirst()).filter { Double($1.ms - $0.ms) > 1.5 * frame }.map(\.1.ms)
			// Samples are display frames only where the display link runs at
			// its rate: a Mac with a display, not a CI virtual machine.
			if frame <= 1000.0 / 119 {
				XCTAssertEqual(late, [], "\(step): late frames at these ms")
			} else {
				logMotionCheck("\(step): late frames not checked at \(Int(1000 / frame)) Hz (\(late.count) gaps)")
			}
		}
	}

	/// One frame of the main display, in milliseconds.
	private static var displayFrameMilliseconds: Double {
		1000 / Double(max(NSScreen.main?.maximumFramesPerSecond ?? 60, 1))
	}

	/// Spawn and despawn happen in place: the card's frame never changes,
	/// it fades and scales to its end state, and a re-open mid-despawn turns
	/// back without going blank or jumping.
	func testTheCardSpawnsAndDespawnsInPlace() throws {
		defer { app.launchEnvironment["GENTBOX_REDUCE_MOTION"] = nil }
		let frames = try traceFrames(in: "spawn-in-place")
		let steps = try traceSteps(in: "spawn-in-place")
		XCTAssertLessThanOrEqual(
			Double(steps["spawn"]?.latencyMs ?? .max), 2 * Self.displayFrameMilliseconds + 1, "the panel's first frame came late",
		)
		let byStep = Dictionary(grouping: frames) { $0.step }
		for step in Self.spawnSteps {
			let frames = try XCTUnwrap(byStep[step], step)
			XCTAssertEqual(Set(frames.map(\.panelX)).count, 1, "\(step): the card slid")
			XCTAssertEqual(Set(frames.map(\.panelTop)).count, 1, "\(step): the card's top moved")
			XCTAssertEqual(Set(frames.map(\.panelHeight)).count, 1, "\(step): the card's height changed")
			XCTAssertGreaterThan(frames.first?.panelHeight ?? 0, 60, "\(step): the card opened at its content's height")
		}
		let despawn = try XCTUnwrap(byStep["despawn"])
		XCTAssertEqual(despawn.last?.alpha ?? 1, 0, accuracy: 0.01, "despawn did not fade out")
		let spawn = try XCTUnwrap(byStep["spawn"])
		XCTAssertLessThan(spawn.first?.alpha ?? 1, 1, "spawn did not fade in")
		// The trace runs with full motion: the card scales up from 0.96.
		XCTAssertLessThan(spawn.map(\.zoom).min() ?? 1, 0.999, "spawn did not scale")
		XCTAssertLessThan(despawn.map(\.zoom).min() ?? 1, 0.999, "despawn did not scale")
		// Faded, not snapped: a frame between the start and the end, unless
		// the first sample came only after the fade was over.
		for (step, duration) in [("despawn", 120), ("spawn", 180)] {
			let frames = try XCTUnwrap(byStep[step])
			let latency = steps[step]?.latencyMs ?? 0
			guard latency < duration else {
				logMotionCheck("\(step): not checked, the first sample came \(latency) ms after the change")
				continue
			}
			let between = frames.filter { 0.02 < $0.alpha && $0.alpha < 0.98 }
			XCTAssertFalse(between.isEmpty, "\(step) snapped: no frame between transparent and opaque")
			logMotionCheck("\(step): checked, \(between.count) frames mid-fade")
		}
		XCTAssertEqual(spawn.last?.alpha ?? 0, 1, accuracy: 0.01)
		XCTAssertEqual(spawn.last?.zoom ?? 0, 1, accuracy: 0.001)
		let reopen = try XCTUnwrap(byStep["reopen-mid-despawn"])
		XCTAssertGreaterThan(reopen.map(\.alpha).min() ?? 0, 0.05, "the re-open went blank first")
		for (previous, frame) in zip(reopen, reopen.dropFirst()) {
			XCTAssertLessThanOrEqual(abs(frame.alpha - previous.alpha), 0.5, "the re-open jumped at \(frame.ms) ms")
		}
		XCTAssertEqual(reopen.last?.alpha ?? 0, 1, accuracy: 0.01)
	}

	/// Expanding a row, measured by the app's `Expand` signpost: from the
	/// click to the next display frame, within two display frames each time.
	/// Instruments and Xcode show the same spans.
	func testExpandingARowIsMeasured() throws {
		app.terminate()
		let log = stateFolder.deletingLastPathComponent().appendingPathComponent("signposts.csv")
		try? FileManager.default.removeItem(at: log)
		app.launchEnvironment["GENTBOX_SIGNPOST_LOG"] = log.path
		defer { app.launchEnvironment["GENTBOX_SIGNPOST_LOG"] = nil }
		if !app.launchArguments.contains("-GentboxOpenInboxAtLaunch") {
			app.launchArguments += ["-GentboxOpenInboxAtLaunch", "YES"]
		}
		let launchedAt = Date()
		app.launch()
		try waitForTheHub(startedAfter: launchedAt)
		let titles = (1...3).map { "Release check \($0) passed" }
		for (index, title) in titles.enumerated() {
			try post(["type": "task_completed", "title": title, "conversation": ["id": "perf-\(index)"], "body": "line one\nline two"])
		}
		XCTAssertTrue(element("message:\(titles[0])").waitForExistence(timeout: 10))
		let expand = XCTOSSignpostMetric(subsystem: "com.eiluviann.gentbox", category: "Interaction", name: "Expand")
		let options = XCTMeasureOptions()
		options.iterationCount = 5
		var next = 0
		measure(metrics: [expand], options: options) {
			let title = titles[next % titles.count]
			next += 1
			let row = element("message:\(title)")
			if row.waitForExistence(timeout: 2) {
				row.click()
			}
			_ = element("card:\(title)").waitForExistence(timeout: 2)
		}
		let expands = try String(contentsOf: log, encoding: .utf8)
			.split(separator: "\n").filter { $0.hasPrefix("Expand,") }
			.compactMap { Double($0.split(separator: ",").last ?? "") }
		XCTAssertGreaterThanOrEqual(expands.count, 5, "the Expand signpost was not recorded")
		let bound = 2 * Self.displayFrameMilliseconds + 1
		XCTAssertEqual(expands.filter { $0 > bound }, [], "expands longer than two display frames (\(bound) ms)")
	}

	/// The panel's window reaches the bottom of the screen, transparent below
	/// the card: a click there passes through to what is below, so it counts
	/// as a click elsewhere and closes the panel, as on a menu. It clicks the
	/// desktop, so it runs where the menu bar shows (CI), not on a Mac whose
	/// menu bar hides, where that would click into someone's work.
	func testAClickBelowTheCardPassesThrough() throws {
		try XCTSkipIf(Self.isMenuBarAutoHidden, "clicks the desktop below the panel; runs on CI")
		try post(["type": "task_completed", "title": "Click-through check"])
		openInbox()
		let row = element("message:Click-through check")
		XCTAssertTrue(row.waitForExistence(timeout: 10))
		// A system dialog, not a window, to accessibility (it never tiles).
		let panel = element("gentbox.inbox")
		XCTAssertTrue(panel.exists)
		// Well below the card's last row, inside the panel's window.
		row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 1)).withOffset(CGVector(dx: 0, dy: 140)).click()
		XCTAssertTrue(row.waitForNonExistence(timeout: 5), "the click below the card did not pass through")
	}

	/// With Reduce Motion the card only fades: no frame is scaled.
	func testWithReduceMotionTheCardOnlyFades() throws {
		app.launchEnvironment["GENTBOX_REDUCE_MOTION"] = "1"
		defer { app.launchEnvironment["GENTBOX_REDUCE_MOTION"] = nil }
		let frames = try traceFrames(in: "reduce-motion", reduceMotion: true)
		let spawning = frames.filter { Self.spawnSteps.contains($0.step) }
		XCTAssertFalse(spawning.isEmpty)
		XCTAssertEqual(spawning.map(\.zoom).min() ?? 0, 1, accuracy: 0.0001, "a frame was scaled")
		XCTAssertLessThan(spawning.map(\.alpha).min() ?? 1, 0.5, "the card did not fade")
	}

	private static let spawnSteps: Set<String> = ["despawn", "spawn", "reopen-mid-despawn"]

	/// Says in the log and the result bundle whether a step's motion was
	/// checked, so a green run shows it.
	private func logMotionCheck(_ line: String) {
		print("Motion check · \(line)")
		XCTContext.runActivity(named: "Motion check · \(line)") { _ in }
	}
	/// `Tokens.motionDuration`, which the UI tests cannot import.
	private static let motionMilliseconds = 300.0

	/// The panel never moves: from opening to closing its top edge and left
	/// edge stay where they were set, and only the bottom edge moves.
	///
	/// Each opening places the panel afresh under the status item, whose
	/// place follows the count, so the edges are checked per opening.
	func testThePanelsTopEdgeNeverMoves() throws {
		let frames = try traceFrames(in: "top-edge-never-moves")
		XCTAssertGreaterThan(frames.count, 30)
		let openings = framesByOpening(frames)
		XCTAssertGreaterThan(openings.count, 1, "the trace re-opens the panel")
		for (number, opening) in openings.enumerated() {
			XCTAssertEqual(Set(opening.map(\.panelTop)).count, 1, "opening \(number): the top edge moved: \(Set(opening.map(\.panelTop)))")
			XCTAssertEqual(Set(opening.map(\.panelX)).count, 1, "opening \(number): the panel slid: \(Set(opening.map(\.panelX)))")
		}
	}

	/// The card's keys work in the panel, which never activates the app: ↩
	/// approves, ⌘⌫ denies, R opens the reply field, where typing and ↩ send.
	/// Close on another thread's row filters the list to that thread: the
	/// prompt's card, hidden now, answers no key (B1).
	func testACardTheFilterHidesTakesNoKeys() throws {
		let prompt = try post([
			"type": "decision_required", "blocking": "blocking", "awaitsResponse": true, "title": "Allow editing x.ts?",
			"conversation": ["id": "hidden-1"], "topic": ["key": "web", "title": "Web"], "thread": ["key": "tx", "title": "Thread X"],
			"actions": [["id": "allow", "label": "Approve", "kind": "approve"], ["id": "deny", "label": "Deny", "kind": "deny"]],
		])
		for index in 1...2 {
			try post([
				"type": "task_completed", "title": "Other thread task \(index)", "conversation": ["id": "hidden-\(index + 1)"],
				"topic": ["key": "web", "title": "Web"], "thread": ["key": "ty", "title": "Thread Y"],
			])
		}
		let waiter = try longPoll(prompt)
		defer { waiter.terminate() }
		openInbox()
		let card = element("card:Allow editing x.ts?")
		if !card.waitForExistence(timeout: 3) {
			element("message:Allow editing x.ts?").click()
		}
		XCTAssertTrue(element("action:Approve").waitForExistence(timeout: 10))
		element("message:Other thread task 1").rightClick()
		app.menuItems["Close thread"].click()
		XCTAssertTrue(element("inbox.filter").waitForExistence(timeout: 5))
		XCTAssertTrue(card.waitForNonExistence(timeout: 5), "the filter left the prompt's card expanded")
		app.typeKey(.return, modifierFlags: [])
		Thread.sleep(forTimeInterval: 1.5)
		XCTAssertTrue(waiter.isRunning, "a key answered the card the filter hides")
	}

	/// ↩ twice in quick succession answers the card that showed, not the one
	/// that takes its place before it is seen (B2).
	func testASecondReturnDoesNotAnswerTheNextCardUnseen() throws {
		var waiters: [Process] = []
		for (index, title) in ["Allow editing first.ts?", "Allow editing second.ts?"].enumerated() {
			let id = try post([
				"type": "decision_required", "blocking": "blocking", "awaitsResponse": true, "title": title,
				"conversation": ["id": "twice-\(index)"],
				"actions": [["id": "allow", "label": "Approve", "kind": "approve"], ["id": "deny", "label": "Deny", "kind": "deny"]],
			])
			waiters.append(try longPoll(id))
		}
		defer { waiters.forEach { $0.terminate() } }
		openInbox()
		let titles = ["Allow editing first.ts?", "Allow editing second.ts?"]
		let shown = try XCTUnwrap(titles.first { element("card:\($0)").waitForExistence(timeout: 5) }, "no card expanded")
		let card = element("card:\(shown)")
		XCTAssertTrue(element("action:Approve").waitForExistence(timeout: 5))
		card.coordinate(withNormalizedOffset: CGVector(dx: 0.97, dy: 0.9)).click()
		Thread.sleep(forTimeInterval: 0.4)
		app.typeKey(.return, modifierFlags: [])
		app.typeKey(.return, modifierFlags: [])
		XCTAssertTrue(card.waitForNonExistence(timeout: 5))
		Thread.sleep(forTimeInterval: 1.5)
		let stillWaiting = waiters.filter(\.isRunning).count
		XCTAssertEqual(stillWaiting, 1, "the second ↩ answered the card that had just taken the first one's place")
	}

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

	/// A newer message that supersedes the expanded card takes its place, and
	/// ↩ answers it, never the next-up prompt of another conversation.
	func testASupersededCardStaysAndReturnAnswersItsReplacement() throws {
		let actions: [[String: Any]] = [
			["id": "allow", "label": "Approve", "kind": "approve"],
			["id": "deny", "label": "Deny", "kind": "deny"],
		]
		let other = try post([
			"type": "decision_required", "blocking": "blocking", "awaitsResponse": true,
			"title": "Allow editing a.ts?", "conversation": ["id": "other"], "actions": actions,
		])
		let otherWaiter = try longPoll(other)
		defer { otherWaiter.terminate() }
		// No waiter yet: a successor of a waited-on prompt would only fold into it.
		try post([
			"type": "decision_required", "blocking": "blocking", "awaitsResponse": true,
			"title": "Allow the deploy?", "conversation": ["id": "deploy"], "correlationKey": "deploy", "actions": actions,
		])
		openInbox()
		// The other prompt's waiter must be live, so it is next-up: open it
		// until its answers show (a waiter that attached after opening left
		// the deploy prompt pinned as the card instead).
		let otherCard = element("card:Allow editing a.ts?")
		if !otherCard.waitForExistence(timeout: 5) {
			element("message:Allow editing a.ts?").click()
		}
		XCTAssertTrue(otherCard.waitForExistence(timeout: 5))
		XCTAssertTrue(element("action:Approve").waitForExistence(timeout: 10), "the other prompt's waiter never attached")
		// Then the deploy prompt is the card you read.
		element("message:Allow the deploy?").click()
		XCTAssertTrue(element("card:Allow the deploy?").waitForExistence(timeout: 5))

		let replacement = try post([
			"type": "decision_required", "blocking": "blocking", "awaitsResponse": true,
			"title": "Allow the deploy to staging?", "conversation": ["id": "deploy"], "correlationKey": "deploy",
			"actions": actions,
		])
		let waiter = try longPoll(replacement)
		defer { waiter.terminate() }
		let card = element("card:Allow the deploy to staging?")
		XCTAssertTrue(card.waitForExistence(timeout: 5), "the card did not follow its replacement")
		XCTAssertTrue(element("action:Approve").waitForExistence(timeout: 5))
		card.coordinate(withNormalizedOffset: CGVector(dx: 0.97, dy: 0.9)).click()
		app.typeKey(.return, modifierFlags: [])

		waiter.waitUntilExit()
		let answer = String(decoding: (waiter.standardOutput as? Pipe)?.fileHandleForReading.readDataToEndOfFile() ?? Data(), as: UTF8.self)
		XCTAssertTrue(answer.contains("\"allow\""), answer)
		XCTAssertTrue(otherWaiter.isRunning, "↩ answered the other prompt")
		XCTAssertTrue(element("message:Allow editing a.ts?").exists || element("card:Allow editing a.ts?").exists)
	}

	// MARK: Helpers

	private struct TraceFrame {
		let number: Int
		let step: String
		let ms: Int
		let panelX: Int
		let panelTop: Int
		let panelHeight: Int
		/// Nil when the frame shows no content bottom marker.
		let gap: Double?
		/// Device pixels per point.
		let scale: Double
		/// The card's opacity and scale as presented: spawn and despawn.
		let alpha: Double
		let zoom: Double
	}

	/// The most the panel's height moved between the sample at `index` and
	/// the samples within one 60 Hz frame of it, or its two neighbours.
	private func neighbouringMotion(in frames: [TraceFrame], at index: Int) -> Double {
		let sample = frames[index]
		let neighbours = frames.indices.filter { other in
			abs(other - index) == 1 || abs(frames[other].ms - sample.ms) <= Self.slowestFrameMilliseconds
		}
		return Double(neighbours.map { abs(frames[$0].panelHeight - sample.panelHeight) }.max() ?? 0)
	}

	/// One frame at 60 Hz, rounded up.
	private static let slowestFrameMilliseconds = 17

	/// The frames split where the panel opens again after it closed. A
	/// re-open mid-despawn turns back where it is, so it stays in its
	/// opening.
	private func framesByOpening(_ frames: [TraceFrame]) -> [[TraceFrame]] {
		var openings: [[TraceFrame]] = []
		var previousStep: String?
		for frame in frames {
			if openings.isEmpty || (frame.step != previousStep && frame.step == "spawn") {
				openings.append([])
			}
			openings[openings.count - 1].append(frame)
			previousStep = frame.step
		}
		return openings
	}

	/// `steps.csv`: per step, the card's height before the action and how
	/// long after the change the first sample came.
	private func traceSteps(in name: String) throws -> [String: (beforeHeight: Int, latencyMs: Int)] {
		let csv = traceFolder(name).appendingPathComponent("steps.csv")
		let rows = try String(contentsOf: csv, encoding: .utf8).split(separator: "\n").dropFirst()
		return Dictionary(uniqueKeysWithValues: rows.compactMap { row in
			let fields = row.split(separator: ",").map(String.init)
			guard fields.count == 3, let before = Int(fields[1]), let latency = Int(fields[2]) else { return nil }
			return (fields[0], (before, latency))
		})
	}

	/// Each trace test keeps its own record, beside the state folder the app
	/// empties at every launch.
	private func traceFolder(_ name: String) -> URL {
		stateFolder.deletingLastPathComponent()
			.appendingPathComponent("UITraces", isDirectory: true)
			.appendingPathComponent(name, isDirectory: true)
	}

	/// Launches the app with `AnimationTrace` over five items, answers each
	/// `awaiting-<step>` it writes (a prompt arrives, the expanded item is
	/// superseded with a shorter body, a waiter starts on the prompt), and
	/// reads its record.
	private func traceFrames(in name: String, reduceMotion: Bool = false) throws -> [TraceFrame] {
		app.terminate()
		let trace = traceFolder(name)
		app.launchEnvironment["GENTBOX_REDUCE_MOTION"] = reduceMotion ? "1" : "0"
		traceInUse = trace
		app.launchEnvironment["GENTBOX_TRACE_DIR"] = trace.path
		if !app.launchArguments.contains("-GentboxOpenInboxAtLaunch") {
			app.launchArguments += ["-GentboxOpenInboxAtLaunch", "YES"]
		}
		let launchedAt = Date()
		app.launch()
		try waitForTheHub(startedAfter: launchedAt)
		for index in 1...5 {
			try post(completedTask(index, body: "line one\nline two\nline three"))
		}
		try awaitTrace("arrival", in: trace)
		let prompt = try post([
			"type": "decision_required", "blocking": "blocking", "awaitsResponse": true,
			"title": "Allow the deploy?", "body": "make deploy",
			"conversation": ["id": "session-7"],
			"topic": ["key": "web", "title": "Web"],
			"thread": ["key": "t7", "title": "Thread 7"],
			"actions": [["id": "allow", "label": "Approve", "kind": "approve"], ["id": "deny", "label": "Deny", "kind": "deny"]],
		])
		let expanded = try awaitTrace("update", in: trace)
		if let index = Int(expanded.split(separator: " ").last ?? "") {
			// A new title: the same one within 10 s would only count a repeat.
			try post(completedTask(index, body: "line one", title: "Completed task number \(index), again"))
		}
		try awaitTrace("waiter", in: trace)
		let waiter = try longPoll(prompt)
		defer { waiter.terminate() }
		let csv = trace.appendingPathComponent("frames.csv")
		let deadline = Date().addingTimeInterval(60)
		while !FileManager.default.fileExists(atPath: csv.path), Date() < deadline {
			Thread.sleep(forTimeInterval: 0.5)
		}
		return try String(contentsOf: csv, encoding: .utf8)
			.split(separator: "\n").dropFirst()
			.map { $0.split(separator: ",").map(String.init) }
			.compactMap { fields in
				guard fields.count == 11, let number = Int(fields[0]), let ms = Int(fields[2]), let x = Int(fields[3]),
					let top = Int(fields[4]), let height = Int(fields[5]), let scale = Double(fields[8]),
					let alpha = Double(fields[9]), let zoom = Double(fields[10])
				else { return nil }
				return TraceFrame(
					number: number, step: fields[1], ms: ms, panelX: x, panelTop: top, panelHeight: height,
					gap: Double(fields[7]), scale: scale, alpha: alpha, zoom: zoom,
				)
			}
	}

	/// One of the trace's items; posted again, it supersedes the first.
	private func completedTask(_ index: Int, body: String, title: String? = nil) -> [String: Any] {
		[
			"type": "task_completed", "title": title ?? "Completed task number \(index)", "body": body,
			"conversation": ["id": "session-\(index)"], "correlationKey": "task-\(index)",
			"topic": ["key": "web", "title": "Web"],
			"thread": ["key": "t\(index)", "title": "Thread \(index)"],
		]
	}

	/// Waits for the trace's `awaiting-<step>` and returns what it holds.
	@discardableResult
	private func awaitTrace(_ step: String, in trace: URL) throws -> String {
		let file = trace.appendingPathComponent("awaiting-\(step)")
		let deadline = Date().addingTimeInterval(40)
		while !FileManager.default.fileExists(atPath: file.path), Date() < deadline {
			Thread.sleep(forTimeInterval: 0.1)
		}
		return try String(contentsOf: file, encoding: .utf8)
	}

	/// Keeps `frames.csv` and every frame in the result bundle when the test
	/// failed.
	private func attachTraceOnFailure(from trace: URL) {
		guard (testRun?.failureCount ?? 0) > 0,
			let files = try? FileManager.default.contentsOfDirectory(at: trace, includingPropertiesForKeys: nil)
		else { return }
		for file in files.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
			let attachment: XCTAttachment
			switch file.pathExtension {
			case "csv": attachment = XCTAttachment(contentsOfFile: file)
			case "png":
				guard let image = NSImage(contentsOf: file) else { continue }
				attachment = XCTAttachment(image: image)
			default: continue
			}
			attachment.name = file.lastPathComponent
			attachment.lifetime = .keepAlways
			add(attachment)
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
	/// agent would: 201 for a new message (superseding ones too), 200 for a
	/// repeat or a fold.
	@discardableResult
	private func post(_ fields: [String: Any], expecting status: String = "201") throws -> String {
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
		XCTAssertEqual(lines.last.map(String.init), status, "POST /v1/messages answered \(printed)")
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
