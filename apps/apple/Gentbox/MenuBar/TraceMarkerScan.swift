#if DEBUG
import AppKit

/// Finds `AnimationTraceMarker` in a captured frame. On a 1× display a
/// one-point line that falls between pixel rows blends into the rows around
/// it (green 0.49 and 0.72 on white), so magenta is told by red and blue
/// dominating green, not by fixed levels, and three columns are read.
enum TraceMarkerScan {
	/// Points from the frame's bottom edge up to the marker's lowest row, or
	/// nil when no column shows the marker.
	static func markerHeightAboveBottom(in bitmap: NSBitmapImageRep) -> CGFloat? {
		let scale = CGFloat(bitmap.pixelsHigh) / max(bitmap.size.height, 1)
		let columns = [1, 2, 3].map { bitmap.pixelsWide * $0 / 4 }
		let lowestRows = columns.compactMap { lowestMagentaRow(in: bitmap, column: $0) }
		guard let row = lowestRows.max() else { return nil }
		return CGFloat(bitmap.pixelsHigh - 1 - row) / scale
	}

	static func isMagenta(_ color: NSColor) -> Bool {
		guard let color = color.usingColorSpace(.sRGB) else { return false }
		return min(color.redComponent, color.blueComponent) - color.greenComponent > 0.2
	}

	private static func lowestMagentaRow(in bitmap: NSBitmapImageRep, column: Int) -> Int? {
		stride(from: bitmap.pixelsHigh - 1, through: 0, by: -1).first { row in
			bitmap.colorAt(x: column, y: row).map(isMagenta) ?? false
		}
	}
}
#endif
