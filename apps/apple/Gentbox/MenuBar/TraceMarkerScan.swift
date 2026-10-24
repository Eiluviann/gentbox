#if DEBUG
import AppKit

/// Finds `AnimationTraceMarker` in a captured frame. On a 1× display the
/// line blends into the pixel rows it straddles (green 0.49 and 0.72 on
/// white), so magenta is told by red and blue dominating green, above a
/// floor that keeps a pink or purple accent colour out, and the marker is
/// placed by the centre of its rows, read in three columns.
enum TraceMarkerScan {
	/// Points from the frame's bottom edge up to the marker's centre, or nil
	/// when no column shows the marker.
	static func markerCentreAboveBottom(in bitmap: NSBitmapImageRep) -> CGFloat? {
		let scale = CGFloat(bitmap.pixelsHigh) / max(bitmap.size.height, 1)
		let columns = [1, 2, 3].map { bitmap.pixelsWide * $0 / 4 }
		let centres = columns.compactMap { markerCentreRow(in: bitmap, column: $0) }
		guard let centre = centres.max() else { return nil }
		// Row `i` spans [i, i + 1) from the top.
		return (CGFloat(bitmap.pixelsHigh) - (centre + 0.5)) / scale
	}

	static func isMagenta(_ color: NSColor) -> Bool {
		guard let color = color.usingColorSpace(.sRGB) else { return false }
		let red = color.redComponent
		let blue = color.blueComponent
		return red > 0.8 && blue > 0.8 && min(red, blue) - color.greenComponent > 0.2
	}

	/// The centre of the lowest run of marker rows in `column`.
	private static func markerCentreRow(in bitmap: NSBitmapImageRep, column: Int) -> CGFloat? {
		let isMarker = { (row: Int) in bitmap.colorAt(x: column, y: row).map(isMagenta) ?? false }
		guard let bottom = stride(from: bitmap.pixelsHigh - 1, through: 0, by: -1).first(where: isMarker) else {
			return nil
		}
		var top = bottom
		while top > 0, isMarker(top - 1) {
			top -= 1
		}
		return CGFloat(top + bottom) / 2
	}
}
#endif
