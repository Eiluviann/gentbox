import SwiftUI

/// The design's secondary buttons (`.btn` in design/project/an.css): 28
/// points high, 7-point corners, a 1-point control line on the control
/// background, 13-point medium labels. `danger` keeps the system red for the
/// label (Deny), and a keyboard hint follows the label in the tertiary ink.
/// The primary button is the system's own (`View.primaryButton`).
struct DesignButtonStyle: ButtonStyle {
	enum Kind {
		case normal
		case danger
	}

	var kind: Kind = .normal
	var hint: String?
	var size: DesignButtonSize = .regular

	func makeBody(configuration: Configuration) -> some View {
		HStack(spacing: 6) {
			configuration.label
				.foregroundStyle(kind == .danger ? Tokens.danger : .primary)
			if let hint {
				Text(hint)
					.font(.system(size: 11))
					.foregroundStyle(.tertiary)
			}
		}
		.font(.system(size: size == .regular ? 13 : 12, weight: .medium))
		.lineLimit(1)
		.padding(.horizontal, size == .regular ? 12 : 9)
		.frame(height: size == .regular ? 28 : 22)
		.background(Tokens.controlBackground, in: RoundedRectangle(cornerRadius: radius))
		.overlay(RoundedRectangle(cornerRadius: radius).strokeBorder(Tokens.controlLine, lineWidth: 1))
		.opacity(configuration.isPressed ? 0.75 : 1)
		.contentShape(RoundedRectangle(cornerRadius: radius))
	}

	private var radius: CGFloat {
		size == .regular ? 7 : 6
	}
}

/// `.btn` (28 points) or `.btn.sm` (22 points, 12-point labels).
enum DesignButtonSize {
	case regular
	case small
}

/// A primary button's label: the title, then its keyboard hint at 60 %.
struct PrimaryButtonLabel: View {
	let title: String
	var hint: String?

	var body: some View {
		HStack(spacing: 6) {
			Text(title)
			if let hint {
				Text(hint)
					.font(.system(size: 11))
					.opacity(0.6)
					.accessibilityHidden(true)
			}
		}
	}
}

extension View {
	/// The system's prominent button, filled with the user's accent colour
	/// (the user's decision: not a fixed accent fill), at the design's sizes.
	func primaryButton(size: DesignButtonSize = .regular) -> some View {
		buttonStyle(.borderedProminent)
			.controlSize(size == .regular ? .large : .regular)
	}
}
