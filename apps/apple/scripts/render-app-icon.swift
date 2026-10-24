#!/usr/bin/env swift
// Renders the app icon from the Gentbox mark (design: `Main`, the app icon:
// a #1d1d1f rounded square with the mark in white) into
// Gentbox/Assets.xcassets/AppIcon.appiconset. Run it again after a change to
// the mark: `swift apps/apple/scripts/render-app-icon.swift`.
import AppKit

let folder = URL(fileURLWithPath: CommandLine.arguments.first ?? ".")
	.deletingLastPathComponent()
	.appendingPathComponent("../Gentbox/Assets.xcassets/AppIcon.appiconset")
	.standardizedFileURL

/// The macOS icon grid: an 824-point rounded square centred on 1024.
func render(pixels: Int) -> Data? {
	let side = CGFloat(pixels)
	guard let bitmap = NSBitmapImageRep(
		bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8,
		samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
		bytesPerRow: 0, bitsPerPixel: 0,
	) else { return nil }
	NSGraphicsContext.saveGraphicsState()
	defer { NSGraphicsContext.restoreGraphicsState() }
	guard let context = NSGraphicsContext(bitmapImageRep: bitmap) else { return nil }
	NSGraphicsContext.current = context
	let unit = side / 1024
	let tile = NSRect(x: 100 * unit, y: 100 * unit, width: 824 * unit, height: 824 * unit)
	NSColor(red: 0x1D / 255, green: 0x1D / 255, blue: 0x1F / 255, alpha: 1).setFill()
	NSBezierPath(roundedRect: tile, xRadius: 185 * unit, yRadius: 185 * unit).fill()

	// The mark, 100 units square, at 55 % of the tile, as in the design.
	let markSide = 824 * unit * 0.55
	let mark = NSRect(x: tile.midX - markSide / 2, y: tile.midY - markSide / 2, width: markSide, height: markSide)
	let configuration = NSImage.SymbolConfiguration(pointSize: markSide, weight: .regular)
		.applying(NSImage.SymbolConfiguration(paletteColors: [.white]))
	if let bubble = NSImage(systemSymbolName: "bubble.left", accessibilityDescription: nil)?
		.withSymbolConfiguration(configuration)
	{
		let scale = min(mark.width / bubble.size.width, mark.height / bubble.size.height)
		let size = NSSize(width: bubble.size.width * scale, height: bubble.size.height * scale)
		let rect = NSRect(x: mark.midX - size.width / 2, y: mark.midY - size.height / 2, width: size.width, height: size.height)
		bubble.draw(in: rect)
	}
	// The prompt, in the mark's frame with y pointing down.
	let point = { (x: CGFloat, y: CGFloat) in
		NSPoint(x: mark.minX + x / 100 * markSide, y: mark.maxY - y / 100 * markSide)
	}
	NSColor.white.setStroke()
	for stroke in [[(28.0, 24.0), (42.0, 37.5), (28.0, 51.0)], [(50.0, 51.0), (68.0, 51.0)]] {
		let path = NSBezierPath()
		path.lineWidth = 7 / 100 * markSide
		path.lineCapStyle = .round
		path.lineJoinStyle = .round
		for (index, (x, y)) in stroke.enumerated() {
			if index == 0 { path.move(to: point(x, y)) } else { path.line(to: point(x, y)) }
		}
		path.stroke()
	}
	return bitmap.representation(using: .png, properties: [:])
}

var images: [[String: String]] = []
for points in [16, 32, 128, 256, 512] {
	for scale in [1, 2] {
		let pixels = points * scale
		let name = "icon_\(pixels).png"
		if let data = render(pixels: pixels) {
			try data.write(to: folder.appendingPathComponent(name))
		}
		images.append(["idiom": "mac", "scale": "\(scale)x", "size": "\(points)x\(points)", "filename": name])
	}
}
let contents: [String: Any] = ["images": images, "info": ["author": "xcode", "version": 1]]
let json = try JSONSerialization.data(withJSONObject: contents, options: [.prettyPrinted, .sortedKeys])
try json.write(to: folder.appendingPathComponent("Contents.json"))
print("Rendered \(images.count) icons into \(folder.path)")
