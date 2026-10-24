#if DEBUG
import AppKit
import GentboxCore
import SwiftUI

/// A debug hook for the accordion's animation: with `GENTBOX_TRACE_DIR` set
/// (and the inbox opened at launch), it drives expand, collapse,
/// expand-another and a refresh mid-animation exactly as a click does, and
/// records every display frame for a moment after each step: the panel's
/// frame, the content's laid-out height, and a PNG of what the panel draws.
/// It never clicks or types, so it steals no focus.
@MainActor
final class AnimationTrace {
	private let directory: URL
	private let panel: NSPanel
	private let measure: () -> CGFloat
	private let inbox: () -> InboxModel?
	private var log = "frame,step,ms,panelY,panelHeight,contentHeight\n"
	private var frame = 0
	/// Captured in memory and written after each step, so encoding never
	/// slows the frames being recorded.
	private var captures: [(String, NSBitmapImageRep)] = []

	/// - Parameter measure: the content's ideal height now.
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
			let items = model.visibleItems.map(\.id)
			await step("expand-row") { model.toggleExpansion(of: items[2]) }
			await step("collapse") { model.toggleExpansion(of: items[2]) }
			await step("expand-another") { model.toggleExpansion(of: items[3]) }
			await step("expand-then-refresh") {
				model.toggleExpansion(of: items[1])
				Task { @MainActor in
					try? await Task.sleep(for: .milliseconds(120))
					model.requestRead()
				}
			}
			try? log.write(to: directory.appendingPathComponent("frames.csv"), atomically: true, encoding: .utf8)
			FileHandle.standardError.write(Data("TRACE done: \(frame) frames in \(directory.path)\n".utf8))
		}
	}

	/// Performs `action`, then records a frame every 1/60 s for 0.9 s.
	private func step(_ name: String, _ action: () -> Void) async {
		let start = Date()
		action()
		while Date().timeIntervalSince(start) < 0.9 {
			record(name, at: Date().timeIntervalSince(start))
			try? await Task.sleep(for: .milliseconds(16))
		}
		for (file, bitmap) in captures {
			try? bitmap.representation(using: .png, properties: [:])?.write(to: directory.appendingPathComponent(file))
		}
		captures.removeAll()
		try? await Task.sleep(for: .milliseconds(300))
	}

	private func record(_ name: String, at seconds: TimeInterval) {
		frame += 1
		let contentHeight = measure()
		log += "\(frame),\(name),\(Int(seconds * 1000)),\(Int(panel.frame.minY)),\(Int(panel.frame.height)),\(Int(contentHeight))\n"
		guard let view = panel.contentView,
			let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)
		else { return }
		view.cacheDisplay(in: view.bounds, to: bitmap)
		captures.append((String(format: "%04d-%@.png", frame, name), bitmap))
	}
}
#endif
