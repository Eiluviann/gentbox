import AppKit
import XCTest

/// The menu-bar mark is a template image (the system tints it for light and
/// dark menu bars) except while agents wait, when its dot is red.
@MainActor
final class MenuBarMarkTests: XCTestCase {
	func testIdleAndUnreadAreTemplatesAndBlockedIsNot() {
		XCTAssertTrue(MenuBarMark.image(for: .idle).isTemplate)
		XCTAssertTrue(MenuBarMark.image(for: .unread).isTemplate)
		XCTAssertFalse(MenuBarMark.image(for: .blocked).isTemplate)
	}

	func testTheMarkFitsTheMenuBar() {
		XCTAssertEqual(MenuBarMark.image(for: .idle).size, NSSize(width: 18, height: 18))
	}

	func testTheBlockedMarkDrawsRed() throws {
		let image = MenuBarMark.image(for: .blocked)
		let bitmap = try XCTUnwrap(render(image))
		// The dot's centre, at (84, 16) of the 100-unit frame, y down.
		let x = Int(Double(bitmap.pixelsWide) * 0.84)
		let y = Int(Double(bitmap.pixelsHigh) * 0.16)
		let color = try XCTUnwrap(bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB))
		XCTAssertGreaterThan(color.redComponent, 0.7)
		XCTAssertLessThan(color.greenComponent, 0.4)
	}

	private func render(_ image: NSImage) -> NSBitmapImageRep? {
		let pixels = 72
		guard let bitmap = NSBitmapImageRep(
			bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8,
			samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
			bytesPerRow: 0, bitsPerPixel: 0,
		) else { return nil }
		NSGraphicsContext.saveGraphicsState()
		NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
		image.draw(in: NSRect(x: 0, y: 0, width: pixels, height: pixels))
		NSGraphicsContext.restoreGraphicsState()
		return bitmap
	}
}
