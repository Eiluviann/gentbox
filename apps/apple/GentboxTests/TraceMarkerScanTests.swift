import AppKit
import XCTest

/// The trace's marker detector reads the frames CI captures on a 1× display,
/// where the marker blends into the pixel rows it straddles.
final class TraceMarkerScanTests: XCTestCase {
	func testABlendedHalfPixelLineOnA1xFrameIsFound() throws {
		// What CI drew: rows 366 and 367 of 380 at green 0.49 and 0.72.
		let frame = try whiteFrame(pixels: NSSize(width: 380, height: 380), points: NSSize(width: 380, height: 380))
		paintRow(366, red: 1, green: 0.49, blue: 1, in: frame)
		paintRow(367, red: 1, green: 0.72, blue: 1, in: frame)
		XCTAssertEqual(TraceMarkerScan.markerHeightAboveBottom(in: frame), 12)
	}

	func testAPointLineOnA2xFrameIsMeasuredInPoints() throws {
		let frame = try whiteFrame(pixels: NSSize(width: 200, height: 200), points: NSSize(width: 100, height: 100))
		paintRow(150, red: 1, green: 0, blue: 1, in: frame)
		paintRow(151, red: 1, green: 0, blue: 1, in: frame)
		XCTAssertEqual(TraceMarkerScan.markerHeightAboveBottom(in: frame), 24)
	}

	func testNoMarkerAndGreyOrRedRowsAreNotOne() throws {
		let frame = try whiteFrame(pixels: NSSize(width: 100, height: 100), points: NSSize(width: 100, height: 100))
		paintRow(40, red: 0.84, green: 0.84, blue: 0.84, in: frame)
		paintRow(60, red: 0.84, green: 0, blue: 0.08, in: frame)
		XCTAssertNil(TraceMarkerScan.markerHeightAboveBottom(in: frame))
	}

	private func whiteFrame(pixels: NSSize, points: NSSize) throws -> NSBitmapImageRep {
		let bitmap = try XCTUnwrap(NSBitmapImageRep(
			bitmapDataPlanes: nil, pixelsWide: Int(pixels.width), pixelsHigh: Int(pixels.height),
			bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
			colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0,
		))
		bitmap.size = points
		for row in 0..<bitmap.pixelsHigh {
			paintRow(row, red: 1, green: 1, blue: 1, in: bitmap)
		}
		return bitmap
	}

	private func paintRow(_ row: Int, red: CGFloat, green: CGFloat, blue: CGFloat, in bitmap: NSBitmapImageRep) {
		let color = NSColor(deviceRed: red, green: green, blue: blue, alpha: 1)
		for column in 0..<bitmap.pixelsWide {
			bitmap.setColor(color, atX: column, y: row)
		}
	}
}
