import AppKit
import QuartzCore

/// One fade and zoom of a view's layer by `MotionSpec`, starting at `start`
/// on the motion's clock: kept, so a later one starts from what this one
/// shows then, and nothing jumps.
struct ContentMotion: Equatable {
	static let opacityKey = "motion.content.opacity"
	static let transformKey = "motion.content.transform"

	let spec: MotionSpec.Content
	let start: TimeInterval

	func opacity(at time: TimeInterval) -> Float {
		spec.fromOpacity + (spec.toOpacity - spec.fromOpacity) * Float(progress(at: time))
	}

	func scale(at time: TimeInterval) -> CGFloat {
		spec.fromScale + (spec.toScale - spec.fromScale) * CGFloat(progress(at: time))
	}

	/// Sets `view` to where the motion ends and animates its layer there
	/// from where it starts, on the motion curve. A zoom that ends smaller
	/// holds there, for a slot that has left and waits to be dropped.
	@MainActor
	func commit(to view: NSView) {
		view.alphaValue = CGFloat(spec.toOpacity)
		guard let layer = view.layer else { return }
		let begin = layer.convertTime(start + spec.delay, from: nil)
		let fade = CABasicAnimation(keyPath: "opacity")
		fade.fromValue = spec.fromOpacity
		fade.toValue = spec.toOpacity
		Self.time(fade, from: begin, over: spec.duration)
		layer.add(fade, forKey: Self.opacityKey)
		guard spec.fromScale != 1 || spec.toScale != 1 else {
			layer.removeAnimation(forKey: Self.transformKey)
			return
		}
		let zoom = CABasicAnimation(keyPath: "transform")
		zoom.fromValue = NSValue(caTransform3D: Self.centredScale(spec.fromScale, in: layer))
		zoom.toValue = NSValue(caTransform3D: Self.centredScale(spec.toScale, in: layer))
		Self.time(zoom, from: begin, over: spec.duration)
		if spec.toScale != 1 {
			zoom.fillMode = .both
			zoom.isRemovedOnCompletion = false
		}
		layer.add(zoom, forKey: Self.transformKey)
	}

	/// A scale about the layer's centre, wherever its anchor is (an AppKit
	/// view's layer is anchored at its corner).
	static func centredScale(_ scale: CGFloat, in layer: CALayer) -> CATransform3D {
		let x = layer.bounds.width * (0.5 - layer.anchorPoint.x)
		let y = layer.bounds.height * (0.5 - layer.anchorPoint.y)
		return CATransform3DConcat(
			CATransform3DConcat(CATransform3DMakeTranslation(-x, -y, 0), CATransform3DMakeScale(scale, scale, 1)),
			CATransform3DMakeTranslation(x, y, 0),
		)
	}

	private func progress(at time: TimeInterval) -> Double {
		MotionCurve.standard.value(at: (time - start - spec.delay) / spec.duration)
	}

	@MainActor
	private static func time(_ animation: CABasicAnimation, from begin: CFTimeInterval, over duration: TimeInterval) {
		animation.beginTime = begin
		animation.duration = duration
		animation.fillMode = .backwards
		animation.timingFunction = Tokens.motionCurve
	}
}

/// One slot of a stack (ADR 0013): it owns its frame, which its
/// `SlotStack` moves on the motion curve, and clips what it holds to it;
/// it is never scaled. What it holds sits in `content`, laid out by the
/// slot's owner at its natural size from the slot's top, and fades and
/// zooms about its own centre as the slot comes and goes.
@MainActor
class SlotView: FlippedView {
	let content = FlippedView()
	/// Closing its place: out of reach of clicks and VoiceOver.
	var isLeaving = false
	private(set) var contentMotion: ContentMotion?

	override init(frame: NSRect) {
		super.init(frame: frame)
		layer?.masksToBounds = true
		addSubview(content)
	}

	@available(*, unavailable)
	required init?(coder _: NSCoder) {
		fatalError("init(coder:) is not used")
	}

	/// What `content` shows at `time`, mid-fade or not.
	func shownContent(at time: TimeInterval) -> (opacity: Float, scale: CGFloat) {
		guard let contentMotion else { return (Float(content.alphaValue), 1) }
		return (contentMotion.opacity(at: time), contentMotion.scale(at: time))
	}

	func moveContent(_ spec: MotionSpec.Content, at time: TimeInterval) {
		let motion = ContentMotion(spec: spec, start: time)
		motion.commit(to: content)
		contentMotion = motion
	}

	/// Drops every motion of the slot and its content: both show their model.
	func endMotion() {
		for layer in [layer, content.layer].compactMap({ $0 }) {
			for key in layer.animationKeys() ?? [] where key.hasPrefix("motion.") {
				layer.removeAnimation(forKey: key)
			}
		}
		contentMotion = nil
	}

	override func hitTest(_ point: NSPoint) -> NSView? {
		isLeaving ? nil : super.hitTest(point)
	}
}
