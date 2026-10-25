import Foundation
import GentboxCore
import Observation
import os

/// The app's state: starting the hub, running with its inbox, or failed
/// with the reason in words.
@MainActor
@Observable
final class AppModel {
	enum Phase {
		case starting
		case running(InboxModel)
		case failed(String)
	}

	private(set) var phase: Phase = .starting
	/// Where the state lives, for the log and the failure line.
	private(set) var stateFolder: StateFolder?
	let loginItem = LoginItem()

	@ObservationIgnored private var hub: GentboxHub?
	@ObservationIgnored private var presence: PresenceMonitor?
	@ObservationIgnored private let log = Logger(subsystem: "com.eiluviann.gentbox", category: "app")

	/// `GENTBOX_STATE_DIR` (and its reset) works in test builds only.
	private static var allowsStateFolderOverride: Bool {
		#if DEBUG || GENTBOX_HOOKS
		true
		#else
		false
		#endif
	}

	/// The inbox once the hub runs.
	var inbox: InboxModel? {
		if case .running(let inbox) = phase { inbox } else { nil }
	}

	/// Starts the hub off the main thread: opening and migrating the store
	/// blocks.
	func start() {
		let folder = StateFolder.resolve(allowsOverride: Self.allowsStateFolderOverride)
		stateFolder = folder
		log.info("state folder \(String(describing: folder.source), privacy: .public) at \(folder.url.path, privacy: .private)")
		let options = HubOptions(stateDir: folder.url.path, homeDir: StateFolder.realHomeFolder().path)
		// Events hop to the main actor before they touch the model; the
		// delegate never calls the hub back synchronously (`HubDelegate`).
		let delegate = MainActorDelegate { [weak self] event in
			self?.inbox?.handle(event: event)
		}
		DispatchQueue.global(qos: .userInitiated).async {
			let result = Result { try GentboxHub.start(options: options, delegate: delegate) }
			DispatchQueue.main.async {
				MainActor.assumeIsolated { self.started(result) }
			}
		}
	}

	/// Stops the hub; blocks until the store is closed. Called at quit.
	func stop() {
		presence?.stop()
		hub?.stop()
	}

	private func started(_ result: Result<GentboxHub, any Error>) {
		switch result {
		case .success(let hub):
			self.hub = hub
			log.info("hub running on port \(hub.port(), privacy: .public)")
			let inbox = InboxModel(service: hub)
			phase = .running(inbox)
			inbox.requestRead()
			startPresence(hub: hub)
		case .failure(let error):
			let words = InboxModel.words(for: error)
			log.error("hub did not start: \(words, privacy: .public)")
			// Also on stderr, for a build started from a terminal.
			FileHandle.standardError.write(Data("gentbox: the hub did not start: \(words)\n".utf8))
			phase = .failed(words)
		}
	}

	private func startPresence(hub: GentboxHub) {
		let monitor = PresenceMonitor { [log] update in
			Task {
				do {
					try await hub.updatePresence(presence: update)
				} catch {
					log.error("presence update failed: \(error.localizedDescription, privacy: .public)")
				}
			}
		}
		presence = monitor
		monitor.start()
	}
}
