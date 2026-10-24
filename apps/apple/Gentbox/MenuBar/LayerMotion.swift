import QuartzCore

/// One layout change, moved by Core Animation: note what each layer shows
/// now (`capture`), set the new layout (view frames, layer properties), then
/// `commit` animates every captured property that changed, or is still on
/// its way, from what was shown to where the model now is. The render server
/// runs the animations, so no frame of the motion touches the main thread,
/// and all of them start together on one curve, as the card's height does.
///
/// A change that lands mid-motion starts from where everything shows, so
/// nothing jumps; every moving property restarts with it, so the pieces stay
/// on one clock.
@MainActor
struct LayerMotion {
	private var captured: [(layer: CALayer, keyPath: String, shown: Any?)] = []

	/// Notes what `layer` shows now for `keyPaths`.
	mutating func capture(_ layer: CALayer?, _ keyPaths: [String]) {
		guard let layer else { return }
		let shown = layer.presentation() ?? layer
		for keyPath in keyPaths {
			captured.append((layer, keyPath, shown.value(forKeyPath: keyPath)))
		}
	}

	/// Animates what changed since `capture`; without `animated`, drops any
	/// motion still running, so the layers show their model at once.
	func commit(animated: Bool, duration: CFTimeInterval = Tokens.motionDuration, timing: CAMediaTimingFunctionName = Tokens.motionTiming) {
		for (layer, keyPath, shown) in captured {
			let key = Self.key(keyPath)
			let target = layer.value(forKeyPath: keyPath)
			guard animated, let shown, !Self.isEqual(shown, target) else {
				layer.removeAnimation(forKey: key)
				continue
			}
			let animation = CABasicAnimation(keyPath: keyPath)
			animation.fromValue = shown
			animation.toValue = target
			animation.duration = duration
			animation.timingFunction = CAMediaTimingFunction(name: timing)
			layer.add(animation, forKey: key)
		}
	}

	/// Fades `layer` to `opacity` (its model) from what it shows, after
	/// `delay`: the card body's reveal and its fast exit.
	static func fade(_ layer: CALayer?, to opacity: Float, duration: CFTimeInterval, delay: CFTimeInterval = 0, from: Float? = nil) {
		guard let layer else { return }
		let shown = from ?? (layer.presentation() ?? layer).opacity
		layer.opacity = opacity
		guard shown != opacity else {
			layer.removeAnimation(forKey: key("opacity"))
			return
		}
		let animation = CABasicAnimation(keyPath: "opacity")
		animation.fromValue = shown
		animation.toValue = opacity
		animation.duration = duration
		animation.timingFunction = CAMediaTimingFunction(name: .easeOut)
		if delay > 0 {
			animation.beginTime = layer.convertTime(CACurrentMediaTime(), from: nil) + delay
			animation.fillMode = .backwards
		}
		layer.add(animation, forKey: key("opacity"))
	}

	private static func key(_ keyPath: String) -> String {
		"motion.\(keyPath)"
	}

	private static func isEqual(_ shown: Any, _ target: Any?) -> Bool {
		guard let target else { return false }
		if let shown = shown as? NSObject, let target = target as? NSObject {
			return shown.isEqual(target)
		}
		// A `CGPath` (a shadow's) bridges to no `NSObject`.
		return CFEqual(shown as CFTypeRef, target as CFTypeRef)
	}
}
