import AppKit
import os

/// Signposted spans for what a person feels: from a click or a key to the
/// next display frame, the first that can show its answer. Instruments'
/// os_signpost track and `XCTOSSignpostMetric` read them
/// (subsystem `com.eiluviann.gentbox`, category `Interaction`).
@MainActor
enum Interactions {
	static let subsystem = "com.eiluviann.gentbox"
	static let category = "Interaction"
	/// The panel whose display frames end the spans.
	static weak var panel: NSPanel?

	/// Begins `name` now and ends it at the panel's next display frame.
	static func untilNextFrame(_ name: StaticString) {
		let state = signposter.beginInterval(name)
		FrameEnd.shared.add(name: name, state: state)
	}

	fileprivate static let signposter = OSSignposter(subsystem: subsystem, category: category)
}

/// Ends the waiting spans at the next frame of a display link that runs only
/// while one is waiting.
@MainActor
private final class FrameEnd: NSObject {
	static let shared = FrameEnd()
	private var waiting: [(name: StaticString, state: OSSignpostIntervalState, start: CFTimeInterval)] = []
	private var link: CADisplayLink?

	func add(name: StaticString, state: OSSignpostIntervalState) {
		waiting.append((name, state, CACurrentMediaTime()))
		guard link == nil, let panel = Interactions.panel else { return }
		let link = panel.displayLink(target: self, selector: #selector(frame(_:)))
		link.add(to: .main, forMode: .common)
		self.link = link
	}

	@objc private func frame(_ link: CADisplayLink) {
		for span in waiting {
			Interactions.signposter.endInterval(span.name, span.state)
			#if DEBUG || GENTBOX_HOOKS
			Self.log(span.name, milliseconds: (CACurrentMediaTime() - span.start) * 1000)
			#endif
		}
		waiting = []
		link.invalidate()
		self.link = nil
	}

	#if DEBUG || GENTBOX_HOOKS
	/// `GENTBOX_SIGNPOST_LOG=<file>`: each span's name and length, a line
	/// each, for tests that bound them.
	private static let logURL = ProcessInfo.processInfo.environment["GENTBOX_SIGNPOST_LOG"]
		.flatMap { $0.isEmpty ? nil : URL(fileURLWithPath: $0) }

	private static func log(_ name: StaticString, milliseconds: Double) {
		guard let logURL else { return }
		let line = Data(String(format: "%@,%.1f\n", "\(name)", milliseconds).utf8)
		if let handle = try? FileHandle(forWritingTo: logURL) {
			handle.seekToEndOfFile()
			handle.write(line)
			try? handle.close()
		} else {
			try? line.write(to: logURL)
		}
	}
	#endif
}
