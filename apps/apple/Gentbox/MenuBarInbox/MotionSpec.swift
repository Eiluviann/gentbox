import CoreGraphics

/// What a slot's content does as a change moves its slot (ADR 0013, step
/// 3): the one table every view reads, so each kind of change looks the
/// same wherever it happens. The geometry always moves on the motion curve
/// over `Tokens.motionDuration`; this is the content inside it, also on the
/// motion curve. Under Reduce Motion the fades stay and the zooms go.
@MainActor
enum MotionSpec {
	/// A content change: what its layer fades and zooms between, after
	/// `delay`, over `duration`.
	struct Content: Equatable {
		let fromOpacity: Float
		let toOpacity: Float
		let fromScale: CGFloat
		let toScale: CGFloat
		let duration: Double
		let delay: Double
	}

	enum Kind {
		/// A slot inserted, or taken back as it leaves.
		case insert
		case remove
		/// A card's body as the card opens and closes.
		case expand
		case collapse
		/// The panel itself.
		case open
		case close
	}

	static func content(_ kind: Kind, reducesMotion: Bool) -> Content {
		let zoomed = reducesMotion ? 1 : Tokens.arrivalScale
		switch kind {
		case .insert:
			return Content(fromOpacity: 0, toOpacity: 1, fromScale: zoomed, toScale: 1, duration: Tokens.spawnDuration, delay: 0)
		case .remove:
			return Content(fromOpacity: 1, toOpacity: 0, fromScale: 1, toScale: zoomed, duration: Tokens.despawnDuration, delay: 0)
		case .expand:
			// In over the motion's second half, once the card has made room.
			let half = Tokens.motionDuration / 2
			return Content(fromOpacity: 0, toOpacity: 1, fromScale: 1, toScale: 1, duration: half, delay: half)
		case .collapse:
			return Content(fromOpacity: 1, toOpacity: 0, fromScale: 1, toScale: 1, duration: Tokens.quickFadeDuration, delay: 0)
		case .open:
			return Content(fromOpacity: 0, toOpacity: 1, fromScale: zoomed, toScale: 1, duration: Tokens.spawnDuration, delay: 0)
		case .close:
			let away = reducesMotion ? 1 : Tokens.departureScale
			return Content(fromOpacity: 1, toOpacity: 0, fromScale: 1, toScale: away, duration: Tokens.despawnDuration, delay: 0)
		}
	}

	/// A leaf whose text or image changed in place, and the icon and
	/// checkbox as picking starts or ends: one crossfade on the motion's
	/// length. Picking and hovering themselves show at once, as pointer
	/// feedback.
	static var crossfadeDuration: Double {
		Tokens.motionDuration
	}
}
