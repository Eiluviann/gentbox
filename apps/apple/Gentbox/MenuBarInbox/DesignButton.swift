import SwiftUI

/// The design's buttons (`.btn` in design/project/an.css): 28 points high,
/// 7-point corners, a 1-point control line on the control background,
/// 13-point medium labels. `primary` fills with the accent, `danger` keeps the
/// system red for the label (Deny), and a keyboard hint follows the label at
/// 60 % on the primary button and in the tertiary ink on the others.
struct DesignButtonStyle: ButtonStyle {
	enum Kind {
		case normal
		case primary
		case danger
	}

	/// `.btn` (28 points) or `.btn.sm` (22 points, 12-point labels).
	enum Size {
		case regular
		case small
	}

	var kind: Kind = .normal
	var hint: String?
	var size: Size = .regular

	func makeBody(configuration: Configuration) -> some View {
		HStack(spacing: 6) {
			configuration.label
				.foregroundStyle(labelColor)
			if let hint {
				Text(hint)
					.font(.system(size: 11))
					.foregroundStyle(kind == .primary ? AnyShapeStyle(Color.white.opacity(0.6)) : AnyShapeStyle(.tertiary))
			}
		}
		.font(.system(size: size == .regular ? 13 : 12, weight: .medium))
		.lineLimit(1)
		.padding(.horizontal, size == .regular ? 12 : 9)
		.frame(height: size == .regular ? 28 : 22)
		.background(background, in: RoundedRectangle(cornerRadius: radius))
		.overlay(RoundedRectangle(cornerRadius: radius).strokeBorder(border, lineWidth: 1))
		.opacity(configuration.isPressed ? 0.75 : 1)
		.contentShape(RoundedRectangle(cornerRadius: radius))
	}

	private var radius: CGFloat {
		size == .regular ? 7 : 6
	}

	private var labelColor: Color {
		switch kind {
		case .normal: .primary
		case .primary: .white
		case .danger: Tokens.danger
		}
	}

	private var background: Color {
		kind == .primary ? .accentColor : Tokens.controlBackground
	}

	private var border: Color {
		kind == .primary ? .accentColor : Tokens.controlLine
	}
}
