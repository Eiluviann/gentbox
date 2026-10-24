import Foundation

/// The hub's delegate on the Swift side: it hops every event to the main
/// actor and returns at once.
///
/// The hub calls `onEvent` on its own thread, inside the actor's turn
/// (see `HubDelegate`). Doing the work there would hold the hub up, and
/// calling back into `GentboxHub` synchronously would deadlock it, so this
/// type only enqueues. `DispatchQueue.main` keeps the events in the order the
/// hub sent them; the handler then calls the hub with `await`, never
/// synchronously.
public final class MainActorDelegate: HubDelegate, Sendable {
	private let handler: @MainActor @Sendable (HubEvent) -> Void

	/// - Parameter handler: runs on the main actor, once per event, in order.
	public init(handler: @escaping @MainActor @Sendable (HubEvent) -> Void) {
		self.handler = handler
	}

	public func onEvent(event: HubEvent) {
		let handler = handler
		DispatchQueue.main.async {
			MainActor.assumeIsolated {
				handler(event)
			}
		}
	}
}
