#if DEBUG || GENTBOX_HOOKS
import AppKit
import GentboxCore

/// A debug hook for the inbox's motion: with `GENTBOX_TRACE_DIR` set (and
/// the inbox opened at launch), it drives the inbox exactly as clicks do
/// and waits for what the UI test sends, then records every display frame
/// for a moment after each step: the panel's left edge, top and height, the
/// content's natural height, where the content's bottom is drawn (`gap`),
/// and a PNG. It never clicks or types, so it steals no focus.
///
/// The steps: expand, collapse, expand another, a refresh mid-motion, a
/// removal; then, each after it writes `awaiting-<step>`, an arrival (the
/// test posts a prompt), an update (the test supersedes the expanded item
/// with a shorter body; the file names its title), expanding the prompt,
/// and a waiter (the test starts a long-poll on the prompt, so its answers
/// replace Snooze).
///
/// Sampling runs on the panel's display link and starts at the first frame
/// drawn after the change. The motion runs in Core Animation, so each sample
/// reads what is presented: the card's height from its mask, and the frame
/// from the content's presentation layers.
///
/// `gap` is the distance in points from the content's bottom, marked by
/// `AnimationTraceMarker`, to the panel's bottom edge, read from the frame
/// the panel draws: 0 when the panel's height moves with the content,
/// positive when the panel is taller, negative when it is shorter and cuts
/// the content off (down to -`AnimationTraceMarker.height`). `missing` when
/// the frame shows no marker at all.
@MainActor
final class AnimationTrace: NSObject {
	private let directory: URL
	private let panel: InboxPanel
	private let measure: () -> CGFloat
	private let inbox: () -> InboxModel?
	private let open: () -> Void
	private let close: () -> Void
	private var log = "frame,step,ms,panelX,panelTop,panelHeight,contentHeight,gap,scale,alpha,zoom\n"
	/// Per step: the card's height before the action, and how long after
	/// the change the first sample came.
	private var steps = "step,beforeHeight,latencyMs\n"
	private var frame = 0
	/// Captured in memory and analysed after each step, so the work never
	/// slows the frames being recorded.
	private var captures: [(name: String, line: String, bitmap: NSBitmapImageRep)] = []
	private var link: CADisplayLink?
	private var frameWaiters: [CheckedContinuation<Void, Never>] = []

	/// - Parameter measure: the content's natural height now.
	init(
		directory: URL, panel: InboxPanel, measure: @escaping () -> CGFloat, inbox: @escaping () -> InboxModel?,
		open: @escaping () -> Void, close: @escaping () -> Void,
	) {
		self.directory = directory
		self.panel = panel
		self.measure = measure
		self.inbox = inbox
		self.open = open
		self.close = close
	}

	func run() {
		Task { @MainActor in
			// A fresh record: another run's frames must not mix in. Only ever
			// a test's own `UITraces/<name>` folder is emptied.
			if directory.deletingLastPathComponent().lastPathComponent == "UITraces" {
				try? FileManager.default.removeItem(at: directory)
			}
			try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
			// Wait for an inbox with enough rows.
			while (inbox()?.visibleItems.count ?? 0) < 4 {
				try? await Task.sleep(for: .milliseconds(200))
			}
			try? await Task.sleep(for: .milliseconds(800))
			guard let model = inbox() else { return }
			let items = model.visibleItems
			await step("expand-row") { model.toggleExpansion(of: items[2].id) }
			await step("collapse") { model.toggleExpansion(of: items[2].id) }
			await step("expand-another") { model.toggleExpansion(of: items[3].id) }
			await step("expand-then-refresh") {
				model.toggleExpansion(of: items[1].id)
				Task { @MainActor in
					try? await Task.sleep(for: .milliseconds(120))
					model.requestRead()
				}
			}
			await step("removal", until: { model.visibleItems.count < items.count }) {
				Task { await model.dismiss(items[0]) }
			}
			let before = model.visibleItems.map(\.id)
			await step("arrival", until: { model.visibleItems.count > before.count }) {
				self.signal("arrival")
			}
			let expandedTitle = model.visibleItems.first { model.isExpanded($0.id) }?.title ?? ""
			await step("update", until: { !model.visibleItems.contains { $0.id == items[1].id } }) {
				self.signal("update", expandedTitle)
			}
			let prompt = model.visibleItems.first { $0.messageType == .decisionRequired }
			if let prompt {
				await step("expand-prompt") {
					if !model.isExpanded(prompt.id) { model.toggleExpansion(of: prompt.id) }
				}
				await step("waiter", until: { model.visibleItems.first { $0.id == prompt.id }?.hasLiveWaiter == true }) {
					self.signal("waiter")
				}
			}
			// Spawn and despawn: the card's frame holds while it fades and
			// scales; a re-open mid-despawn turns it back.
			await step("despawn") { close() }
			await step("spawn") { open() }
			await step("reopen-mid-despawn") {
				close()
				Task { @MainActor in
					try? await Task.sleep(for: .milliseconds(50))
					open()
				}
			}
			link?.invalidate()
			try? steps.write(to: directory.appendingPathComponent("steps.csv"), atomically: true, encoding: .utf8)
			try? log.write(to: directory.appendingPathComponent("frames.csv"), atomically: true, encoding: .utf8)
			FileHandle.standardError.write(Data("TRACE done: \(frame) frames in \(directory.path)\n".utf8))
		}
	}

	/// Tells the UI test to act: `awaiting-<step>`, holding `detail`.
	private func signal(_ step: String, _ detail: String = "") {
		try? Data(detail.utf8).write(to: directory.appendingPathComponent("awaiting-\(step)"))
	}

	/// Performs `action`, waits up to 20 s for `until` (a change that comes
	/// back from the hub), lets that change reach a drawn frame, then
	/// records every display frame for 0.6 s.
	private func step(_ name: String, until isUnderway: () -> Bool = { true }, _ action: () -> Void) async {
		let beforeHeight = panel.presentedCardFrameOnScreen.height
		action()
		let deadline = Date().addingTimeInterval(20)
		while !isUnderway(), Date() < deadline {
			try? await Task.sleep(for: .milliseconds(4))
		}
		let changedAt = Date()
		await nextFrame()
		let start = Date()
		steps += "\(name),\(Int(beforeHeight)),\(Int(start.timeIntervalSince(changedAt) * 1000))\n"
		while Date().timeIntervalSince(start) < 0.6 {
			await nextFrame()
			record(name, at: Date().timeIntervalSince(start))
		}
		for capture in captures {
			let scale = CGFloat(capture.bitmap.pixelsHigh) / max(capture.bitmap.size.height, 1)
			let parts = capture.line.split(separator: "|", maxSplits: 1).map(String.init)
			log += "\(parts[0]),\(Self.gap(in: capture.bitmap)),\(scale),\(parts.count > 1 ? parts[1] : "0,0")\n"
			try? capture.bitmap.representation(using: .png, properties: [:])?
				.write(to: directory.appendingPathComponent(capture.name))
		}
		captures.removeAll()
		try? await Task.sleep(for: .milliseconds(300))
	}

	/// Resumes at the panel's next display frame, or after 20 ms: a panel
	/// that despawned is off screen, and its display link stops.
	private func nextFrame() async {
		if link == nil {
			let link = panel.displayLink(target: self, selector: #selector(displayFrame(_:)))
			link.preferredFrameRateRange = CAFrameRateRange(minimum: 80, maximum: 120, preferred: 120)
			link.add(to: .main, forMode: .common)
			self.link = link
		}
		Task { @MainActor [weak self] in
			try? await Task.sleep(for: .milliseconds(20))
			self?.resumeWaiters()
		}
		await withCheckedContinuation { frameWaiters.append($0) }
	}

	@objc private func displayFrame(_: CADisplayLink) {
		resumeWaiters()
	}

	private func resumeWaiters() {
		let waiters = frameWaiters
		frameWaiters = []
		waiters.forEach { $0.resume() }
	}

	private func record(_ name: String, at seconds: TimeInterval) {
		let card = panel.presentedCardFrameOnScreen
		frame += 1
		let line = [
			"\(frame)", name, "\(Int(seconds * 1000))", "\(Int(card.minX))", "\(Int(card.maxY))",
			"\(Int(card.height))", "\(Int(measure()))",
		].joined(separator: ",")
		let spawn = panel.presentedSpawn
		let spawnColumns = String(format: "%.3f,%.4f", spawn.opacity, spawn.scale)
		// The content as presented now, cut to the card as drawn now.
		guard let bitmap = Self.presented(panel.content, height: card.height, scale: panel.backingScaleFactor)
		else { return }
		captures.append((String(format: "%04d-%@.png", frame, name), line + "|" + spawnColumns, bitmap))
	}

	/// The top `height` points of `view` as its layers show mid-motion: a
	/// view's own drawing (`cacheDisplay`) shows the model, which Core
	/// Animation has already moved to where the motion ends.
	private static func presented(_ view: NSView, height: CGFloat, scale: CGFloat) -> NSBitmapImageRep? {
		guard let layer = view.layer, height > 0,
			let bitmap = NSBitmapImageRep(
				bitmapDataPlanes: nil, pixelsWide: Int(view.bounds.width * scale), pixelsHigh: Int(height * scale),
				bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
				colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0,
			),
			let context = NSGraphicsContext(bitmapImageRep: bitmap)?.cgContext
		else { return nil }
		bitmap.size = NSSize(width: view.bounds.width, height: height)
		context.scaleBy(x: scale, y: scale)
		// Bitmaps are y-up: the content's top, `height` points of it, fills them.
		context.translateBy(x: 0, y: height - view.bounds.height)
		(layer.presentation() ?? layer).render(in: context)
		return bitmap
	}

	/// Points from the content's bottom down to the panel's bottom edge,
	/// from where the marker's centre is drawn, or `missing`.
	private static func gap(in bitmap: NSBitmapImageRep) -> String {
		guard let markerAboveBottom = TraceMarkerScan.markerCentreAboveBottom(in: bitmap) else { return "missing" }
		let centreAboveContentBottom = AnimationTraceMarker.height + AnimationTraceMarker.thickness / 2
		return String(format: "%.2f", markerAboveBottom - centreAboveContentBottom)
	}
}

/// A two-point magenta line `height` points above the content's bottom
/// (over the last row), drawn only while tracing, so a frame shows
/// where the content ends even when the panel cuts a little of it off.
@MainActor
final class AnimationTraceMarker: FlippedView {
	static let isShown = !(ProcessInfo.processInfo.environment["GENTBOX_TRACE_DIR"] ?? "").isEmpty
	static let height: CGFloat = 40
	/// Two points: on a 1× display at least one pixel row is all marker.
	static let thickness: CGFloat = 2

	override init(frame: NSRect) {
		super.init(frame: frame)
		fill = NSColor(srgbRed: 1, green: 0, blue: 1, alpha: 1)
		setAccessibilityElement(false)
	}
}
#endif
