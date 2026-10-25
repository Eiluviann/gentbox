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
	private var captured: [(layer: CALayer, keyPath: String, shown: Any?, isDrawn: Bool)] = []

	/// Notes what `layer` shows now for `keyPaths`: its model, if the render
	/// server has not drawn it yet.
	mutating func capture(_ layer: CALayer?, _ keyPaths: [String]) {
		guard let layer else { return }
		let presentation = layer.presentation()
		let shown = presentation ?? layer
		for keyPath in keyPaths {
			captured.append((layer, keyPath, shown.value(forKeyPath: keyPath), presentation != nil))
		}
	}

	/// Animates what changed since `capture`; without `animated`, drops any
	/// motion still running, so the layers show their model at once.
	func commit(animated: Bool, duration: CFTimeInterval = Tokens.motionDuration, timing: CAMediaTimingFunction = Tokens.motionCurve) {
		for (layer, keyPath, shown, isDrawn) in captured {
			let key = Self.key(keyPath)
			let target = layer.value(forKeyPath: keyPath)
			guard animated, let shown, !Self.isEqual(shown, target) else {
				// Not drawn yet, its model is no sign that it stands still: a
				// second change in the same turn leaves an arrival running.
				if !animated || isDrawn {
					layer.removeAnimation(forKey: key)
				}
				continue
			}
			let animation = CABasicAnimation(keyPath: keyPath)
			animation.fromValue = shown
			animation.toValue = target
			animation.duration = duration
			animation.timingFunction = timing
			layer.add(animation, forKey: key)
		}
	}

	/// Fades `layer` to `opacity` (its model) from what it shows, after
	/// `delay`: the card body's reveal and its fast exit, items and buttons
	/// coming and going.
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
		animation.timingFunction = Tokens.motionCurve
		if delay > 0 {
			animation.beginTime = layer.convertTime(CACurrentMediaTime(), from: nil) + delay
			animation.fillMode = .backwards
		}
		layer.add(animation, forKey: key("opacity"))
	}

	/// Scales `layer` about its centre from `fromScale` (or from what it
	/// shows, mid-zoom) to `toScale`: an item growing into place as it
	/// arrives, or shrinking as it goes. The model stays unscaled; a zoom
	/// that ends smaller holds there until `endZoom` or the layer is hidden.
	static func zoom(_ layer: CALayer?, from fromScale: CGFloat, to toScale: CGFloat, duration: CFTimeInterval) {
		guard let layer else { return }
		let isZooming = layer.animation(forKey: key("transform")) != nil
		let animation = CABasicAnimation(keyPath: "transform")
		animation.fromValue = NSValue(caTransform3D: isZooming ? (layer.presentation() ?? layer).transform : centredScale(fromScale, in: layer))
		animation.toValue = NSValue(caTransform3D: centredScale(toScale, in: layer))
		animation.duration = duration
		animation.timingFunction = Tokens.motionCurve
		if toScale != 1 {
			animation.fillMode = .forwards
			animation.isRemovedOnCompletion = false
		}
		layer.add(animation, forKey: key("transform"))
	}

	/// Drops a zoom: the layer shows unscaled.
	static func endZoom(_ layer: CALayer?) {
		layer?.removeAnimation(forKey: key("transform"))
	}

	/// Opens `layer` up from `height` to its model's height, its top fixed:
	/// an arriving item makes room for itself as a card does when it expands.
	static func grow(_ layer: CALayer?, fromHeight height: CGFloat, duration: CFTimeInterval) {
		guard let layer else { return }
		var from = layer.bounds
		from.size.height = height
		let animation = CABasicAnimation(keyPath: "bounds")
		animation.fromValue = NSValue(rect: from)
		animation.toValue = NSValue(rect: layer.bounds)
		animation.duration = duration
		animation.timingFunction = Tokens.motionCurve
		layer.add(animation, forKey: key("bounds"))
	}

	/// Cross-fades what each of `layers` shows now into what it shows once
	/// this change is drawn: a card whose content changed in place.
	static func crossfade(_ layers: [CALayer?], duration: CFTimeInterval) {
		for layer in layers.compactMap({ $0 }) {
			let transition = CATransition()
			transition.type = .fade
			transition.duration = duration
			transition.timingFunction = Tokens.motionCurve
			layer.add(transition, forKey: key("crossfade"))
		}
	}

	/// A scale about the layer's centre, wherever its anchor is (an AppKit
	/// view's layer is anchored at its corner).
	private static func centredScale(_ scale: CGFloat, in layer: CALayer) -> CATransform3D {
		let x = layer.bounds.width * (0.5 - layer.anchorPoint.x)
		let y = layer.bounds.height * (0.5 - layer.anchorPoint.y)
		return CATransform3DConcat(
			CATransform3DConcat(CATransform3DMakeTranslation(-x, -y, 0), CATransform3DMakeScale(scale, scale, 1)),
			CATransform3DMakeTranslation(x, y, 0),
		)
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
