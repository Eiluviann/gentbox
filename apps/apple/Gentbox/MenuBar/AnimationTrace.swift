#if DEBUG
import AppKit
import GentboxCore
import SwiftUI

/// A debug hook for the inbox's motion: with `GENTBOX_TRACE_DIR` set (and
/// the inbox opened at launch), it drives expand, collapse, expand-another,
/// a refresh mid-animation and the removal of an item exactly as a click
/// does, waits for an item to arrive (the UI test posts one once
/// `awaiting-arrival` appears), and records every display frame for a moment
/// after each step: the panel's left edge, top and height, the content's
/// natural height, where the content's bottom is drawn (`gap`), and a PNG.
/// It never clicks or types, so it steals no focus.
///
/// `gap` is the distance in points from the content's bottom, marked by
/// `AnimationTraceMarker`, to the panel's bottom edge, read from the frame
/// the panel draws: 0 when the panel's height moves with the content,
/// positive when the panel is taller, negative when it is shorter and cuts
/// the content off (down to -`AnimationTraceMarker.height`; beyond that the
/// marker itself is cut off and `gap` reads -99).
@MainActor
final class AnimationTrace {
	private let directory: URL
	private let panel: NSPanel
	private let measure: () -> CGFloat
	private let inbox: () -> InboxModel?
	private var log = "frame,step,ms,panelX,panelTop,panelHeight,contentHeight,gap\n"
	private var frame = 0
	/// Captured in memory and analysed after each step, so the work never
	/// slows the frames being recorded.
	private var captures: [(name: String, line: String, bitmap: NSBitmapImageRep)] = []

	/// - Parameter measure: the content's natural height now.
	init(directory: URL, panel: NSPanel, measure: @escaping () -> CGFloat, inbox: @escaping () -> InboxModel?) {
		self.directory = directory
		self.panel = panel
		self.measure = measure
		self.inbox = inbox
	}

	func run() {
		Task { @MainActor in
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
			let before = model.visibleItems.count
			await step("arrival", until: { model.visibleItems.count > before }) {
				try? Data().write(to: self.directory.appendingPathComponent("awaiting-arrival"))
			}
			try? log.write(to: directory.appendingPathComponent("frames.csv"), atomically: true, encoding: .utf8)
			FileHandle.standardError.write(Data("TRACE done: \(frame) frames in \(directory.path)\n".utf8))
		}
	}

	/// Performs `action`, waits up to 20 s for `until` (a change that comes
	/// back from the hub), then records a frame every 1/60 s for 0.6 s.
	private func step(_ name: String, until isUnderway: () -> Bool = { true }, _ action: () -> Void) async {
		action()
		let deadline = Date().addingTimeInterval(20)
		while !isUnderway(), Date() < deadline {
			try? await Task.sleep(for: .milliseconds(4))
		}
		let start = Date()
		while Date().timeIntervalSince(start) < 0.6 {
			record(name, at: Date().timeIntervalSince(start))
			try? await Task.sleep(for: .milliseconds(16))
		}
		for capture in captures {
			log += "\(capture.line),\(Self.gap(in: capture.bitmap))\n"
			try? capture.bitmap.representation(using: .png, properties: [:])?
				.write(to: directory.appendingPathComponent(capture.name))
		}
		captures.removeAll()
		try? await Task.sleep(for: .milliseconds(300))
	}

	private func record(_ name: String, at seconds: TimeInterval) {
		frame += 1
		let line = [
			"\(frame)", name, "\(Int(seconds * 1000))", "\(Int(panel.frame.minX))", "\(Int(panel.frame.maxY))",
			"\(Int(panel.frame.height))", "\(Int(measure()))",
		].joined(separator: ",")
		// The content's hosting view fills the panel.
		guard let view = panel.contentView?.subviews.first,
			let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)
		else { return }
		view.cacheDisplay(in: view.bounds, to: bitmap)
		captures.append((String(format: "%04d-%@.png", frame, name), line, bitmap))
	}

	/// Points from the marker's lowest row to the bitmap's bottom, or -1.
	private static func gap(in bitmap: NSBitmapImageRep) -> String {
		let scale = CGFloat(bitmap.pixelsHigh) / max(bitmap.size.height, 1)
		let column = bitmap.pixelsWide / 2
		for row in stride(from: bitmap.pixelsHigh - 1, through: 0, by: -1) {
			// Magenta as the display draws it (about 0.92, 0.2, 0.97 in P3).
			guard let color = bitmap.colorAt(x: column, y: row)?.usingColorSpace(.sRGB) else { continue }
			if color.redComponent > 0.8, color.greenComponent < 0.4, color.blueComponent > 0.8 {
				let markerFromBottom = CGFloat(bitmap.pixelsHigh - 1 - row) / scale
				return String(format: "%.1f", markerFromBottom - AnimationTraceMarker.height)
			}
		}
		return "-99"
	}
}

/// A one-point magenta line `height` points above the content's bottom
/// (over the last row), drawn only while tracing, so a frame shows
/// where the content ends even when the panel cuts a little of it off.
struct AnimationTraceMarker: View {
	static let isShown = !(ProcessInfo.processInfo.environment["GENTBOX_TRACE_DIR"] ?? "").isEmpty
	static let height: CGFloat = 40

	var body: some View {
		Color(red: 1, green: 0, blue: 1)
			.frame(height: 1)
			.padding(.bottom, Self.height)
			.accessibilityHidden(true)
	}
}
#endif
