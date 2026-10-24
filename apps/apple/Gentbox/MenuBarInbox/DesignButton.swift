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

	var kind: Kind = .normal
	var hint: String?

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
		.font(.system(size: 13, weight: .medium))
		.lineLimit(1)
		.padding(.horizontal, 12)
		.frame(height: 28)
		.background(background, in: RoundedRectangle(cornerRadius: 7))
		.overlay(RoundedRectangle(cornerRadius: 7).strokeBorder(border, lineWidth: 1))
		.opacity(configuration.isPressed ? 0.75 : 1)
		.contentShape(RoundedRectangle(cornerRadius: 7))
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
