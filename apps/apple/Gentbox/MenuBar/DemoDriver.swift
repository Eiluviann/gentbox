#if DEBUG || GENTBOX_HOOKS
import AppKit
import GentboxCore

/// A debug hook for screen recordings and frame-pacing measurements: with
/// `GENTBOX_DEMO_DIR` set, runs one command per `*.cmd` file dropped there,
/// in name order, and acknowledges each with `<name>.done` (the card's frame,
/// from the top left), so a script drives the inbox with no clicks or keys.
@MainActor
final class DemoDriver: NSObject {
	private let directory: URL
	private let open: () -> Void
	private let close: () -> Void
	private let panel: InboxPanel
	private let inbox: () -> InboxModel?
	/// A plain, click-through surface under the inbox, so a recording shows
	/// nothing of the windows behind it.
	private var backdrop: NSPanel?
	/// Frame pacing: every display frame's timestamp between `pace-start`
	/// and `pace-stop`, on a link that asks ProMotion for 120 Hz.
	private var pacingLink: CADisplayLink?
	private var pacingName = ""
	private var pacingFrames: [(timestamp: CFTimeInterval, target: CFTimeInterval)] = []
	/// Main run loop iterations longer than 4 ms while pacing, from its
	/// start, with their length: what made a frame late.
	private var busyBlocks: [(CFTimeInterval, CFTimeInterval)] = []
	private var busyObserver: CFRunLoopObserver?
	private var awokeAt: CFTimeInterval = 0
	private var pacingStart: CFTimeInterval = 0

	init(directory: URL, panel: InboxPanel, open: @escaping () -> Void, close: @escaping () -> Void, inbox: @escaping () -> InboxModel?) {
		self.directory = directory
		self.panel = panel
		self.open = open
		self.close = close
		self.inbox = inbox
	}

	private var watcher: DispatchSourceFileSystemObject?
	private var isRunning = false

	/// Watches the folder (a kernel event per change, nothing in between, so
	/// the harness never costs the frames it measures) and runs what lands.
	func run() {
		try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
		let descriptor = Darwin.open(directory.path, O_EVTONLY)
		guard descriptor >= 0 else { return }
		let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: descriptor, eventMask: .write, queue: .main)
		source.setEventHandler { [weak self] in
			MainActor.assumeIsolated { self?.drain() }
		}
		source.setCancelHandler { Darwin.close(descriptor) }
		source.resume()
		watcher = source
		drain()
	}

	private func drain() {
		guard !isRunning else { return }
		isRunning = true
		Task { @MainActor in
			defer { isRunning = false }
			while true {
				let files = (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) ?? []
				let commands = files.filter { $0.pathExtension == "cmd" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
				guard !commands.isEmpty else { return }
				for file in commands {
					let command = (try? String(contentsOf: file, encoding: .utf8))?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
					try? FileManager.default.removeItem(at: file)
					await perform(command)
					try? Data(frameLine().utf8).write(to: file.deletingPathExtension().appendingPathExtension("done"))
				}
			}
		}
	}

	/// The panel's frame in screen points from the top left, as
	/// `screencapture -R` takes it.
	private func frameLine() -> String {
		let screenHeight = panel.screen?.frame.height ?? NSScreen.main?.frame.height ?? 0
		let frame = panel.cardFrameOnScreen
		return "\(Int(frame.minX)),\(Int(screenHeight - frame.maxY)),\(Int(frame.width)),\(Int(frame.height))"
	}

	private func startPacing(_ name: String) {
		pacingName = name
		pacingFrames = []
		busyBlocks = []
		pacingStart = CACurrentMediaTime()
		let observer = CFRunLoopObserverCreateWithHandler(nil, CFRunLoopActivity.afterWaiting.rawValue | CFRunLoopActivity.beforeWaiting.rawValue, true, 0) { [weak self] _, activity in
			MainActor.assumeIsolated {
				guard let self else { return }
				let now = CACurrentMediaTime()
				if activity == .afterWaiting {
					self.awokeAt = now
				} else if self.awokeAt > 0, now - self.awokeAt > 0.004 {
					self.busyBlocks.append((self.awokeAt - self.pacingStart, now - self.awokeAt))
				}
			}
		}
		CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes)
		busyObserver = observer
		// Off screen the panel is on no display and its link never fires; the
		// main display's frames time the main thread just the same.
		let screen = StatusItemController.isOffscreen ? NSScreen.main ?? NSScreen.screens.first : nil
		let link = screen?.displayLink(target: self, selector: #selector(pacingFrame(_:)))
			?? panel.displayLink(target: self, selector: #selector(pacingFrame(_:)))
		link.preferredFrameRateRange = CAFrameRateRange(minimum: 80, maximum: 120, preferred: 120)
		link.add(to: .main, forMode: .common)
		pacingLink = link
	}

	@objc private func pacingFrame(_ link: CADisplayLink) {
		pacingFrames.append((link.timestamp, link.targetTimestamp))
	}

	/// Appends `name,frames,refreshHz,hitches,longestMs,intervalsMs,missed,
	/// latestMs` to `pacing.csv`. A hitch is a frame interval over 1.5
	/// refresh intervals, which also counts ProMotion slowing to 48 Hz while
	/// nothing moves; a missed frame is one that came over half a refresh
	/// after the time the frame before it was due, the dropped frame itself.
	private func stopPacing() {
		pacingLink?.invalidate()
		pacingLink = nil
		if let busyObserver { CFRunLoopRemoveObserver(CFRunLoopGetMain(), busyObserver, .commonModes) }
		busyObserver = nil
		let busy = busyBlocks.map { String(format: "%.0f:%.1f", $0.0 * 1000, $0.1 * 1000) }.joined(separator: " ")
		try? Data((pacingName + " " + busy + "\n").utf8).write(to: directory.appendingPathComponent("busy-\(pacingName).txt"))
		let intervals = zip(pacingFrames, pacingFrames.dropFirst()).map { ($1.timestamp - $0.timestamp) * 1000 }
		guard !intervals.isEmpty else { return }
		let refreshMs = intervals.sorted()[intervals.count / 2]
		let hitches = intervals.filter { $0 > refreshMs * 1.5 }.count
		let lateness = zip(pacingFrames, pacingFrames.dropFirst()).map { ($1.timestamp - $0.target) * 1000 }
		let missed = lateness.filter { $0 > refreshMs / 2 }.count
		let line = [
			pacingName, "\(pacingFrames.count)", String(format: "%.0f", 1000 / refreshMs), "\(hitches)",
			String(format: "%.1f", intervals.max() ?? 0),
			intervals.map { String(format: "%.1f", $0) }.joined(separator: " "),
			"\(missed)", String(format: "%.1f", lateness.max() ?? 0),
		].joined(separator: ",") + "\n"
		let file = directory.appendingPathComponent("pacing.csv")
		if let handle = try? FileHandle(forWritingTo: file) {
			handle.seekToEndOfFile()
			handle.write(Data(line.utf8))
			try? handle.close()
		} else {
			try? Data(line.utf8).write(to: file)
		}
	}

	/// `x,y,w,h light|dark` in screen points from the top left, or `off`.
	private func showBackdrop(_ argument: String) {
		backdrop?.orderOut(nil)
		backdrop = nil
		// Off screen there is nothing to hide behind the card.
		guard !StatusItemController.isOffscreen else { return }
		let parts = argument.split(separator: " ")
		let numbers = parts.first?.split(separator: ",").compactMap { Double($0) } ?? []
		guard numbers.count == 4, let screen = panel.screen ?? NSScreen.main else { return }
		let frame = NSRect(
			x: numbers[0], y: screen.frame.height - numbers[1] - numbers[3], width: numbers[2], height: numbers[3],
		)
		let surface = NSPanel(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
		surface.level = NSWindow.Level(rawValue: NSWindow.Level.popUpMenu.rawValue - 1)
		surface.ignoresMouseEvents = true
		surface.collectionBehavior = [.canJoinAllSpaces, .transient, .ignoresCycle, .fullScreenAuxiliary]
		surface.hasShadow = false
		let isDark = parts.count > 1 && parts[1] == "dark"
		surface.backgroundColor = isDark ? NSColor(white: 0.16, alpha: 1) : NSColor(white: 0.86, alpha: 1)
		surface.orderFrontRegardless()
		backdrop = surface
	}

	/// Types `text` into the open reply box as a person would, one change
	/// of the field editor, so the box takes its lines as it does then.
	private func type(_ text: String) {
		guard let editor = panel.firstResponder as? NSTextView, let field = editor.delegate as? NSTextField else {
			inbox()?.updateReply(text)
			return
		}
		editor.string = text
		field.textDidChange(Notification(name: NSText.didChangeNotification, object: editor))
	}

	private func perform(_ command: String) async {
		let parts = command.split(separator: " ", maxSplits: 1).map(String.init)
		let verb = parts.first ?? ""
		let argument = parts.count > 1 ? parts[1] : ""
		let model = inbox()
		let item = model?.visibleItems.first { $0.title == argument }
		switch verb {
		case "open": open()
		case "close": close()
		case "appearance": panel.appearance = NSAppearance(named: argument == "dark" ? .darkAqua : .aqua)
		case "expand":
			if let item { model?.expand(item.id) }
		case "reply":
			if let item { model?.startReply(on: item.id) }
		case "type":
			type(argument)
		case "reduce-motion":
			StatusItemController.reduceMotionOverride = argument == "on"
		case "close-thread":
			if let item { _ = await model?.closeThread(of: item) }
		case "close-filtered": await model?.closeFilteredThread()
		case "dismiss":
			if let item { await model?.dismiss(item) }
		case "select":
			if let item { model?.toggleSelection(of: item) }
		case "dismiss-selected": await model?.dismissSelected()
		case "read": model?.requestRead()
		case "backdrop": showBackdrop(argument)
		case "pace-start": startPacing(argument)
		case "pace-stop": stopPacing()
		default: break
		}
	}
}
#endif
