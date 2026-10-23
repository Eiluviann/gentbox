import AppKit
import CoreText

let N = 24
func normalize(_ draw: (CGContext) -> Void) -> [Float]? {
	let S = 128
	let cs = CGColorSpaceCreateDeviceGray()
	guard let ctx = CGContext(data: nil, width: S, height: S, bitsPerComponent: 8, bytesPerRow: S, space: cs, bitmapInfo: CGImageAlphaInfo.none.rawValue) else { return nil }
	ctx.setFillColor(gray: 0, alpha: 1); ctx.fill(CGRect(x: 0, y: 0, width: S, height: S))
	draw(ctx)
	let buf = ctx.data!.assumingMemoryBound(to: UInt8.self)
	var minX = S, minY = S, maxX = -1, maxY = -1
	for y in 0..<S { for x in 0..<S where buf[y*S+x] > 60 { minX = min(minX,x); maxX = max(maxX,x); minY = min(minY,y); maxY = max(maxY,y) } }
	if maxX < 0 { return nil }
	let w = maxX-minX+1, h = maxY-minY+1
	var out = [Float](repeating: 0, count: N*N+1)
	for j in 0..<N { for i in 0..<N {
		let x = minX + i*w/N, y = minY + j*h/N
		out[j*N+i] = Float(buf[y*S+x])/255
	}}
	out[N*N] = Float(w)/Float(h) * 4
	return out
}

let names = try! String(contentsOfFile: "names.txt", encoding: .utf8).split(separator: "\n").map(String.init)
let puas = try! String(contentsOfFile: "pua.txt", encoding: .utf8).split(separator: "\n").compactMap { UInt32($0) }

var symVecs: [(String,[Float])] = []
for n in names {
	guard let base = NSImage(systemSymbolName: n, accessibilityDescription: nil),
		let img = base.withSymbolConfiguration(.init(pointSize: 72, weight: .regular)) else { print("MISSING \(n)"); continue }
	var r = CGRect(origin: .zero, size: img.size)
	guard let cg = img.cgImage(forProposedRect: &r, context: nil, hints: nil) else { continue }
	let v = normalize { ctx in
		ctx.clip(to: CGRect(x: 4, y: 4, width: r.width, height: r.height), mask: cg)
		ctx.setFillColor(gray: 1, alpha: 1); ctx.fill(CGRect(x: 0, y: 0, width: 128, height: 128))
	}
	if let v { symVecs.append((n, v)) }
}

let font = CTFontCreateWithName("SFPro-Regular" as CFString, 72, nil)
var glyphVecs: [(UInt32,[Float])] = []
for cp in puas {
	var chars = Array(String(UnicodeScalar(cp)!).utf16)
	var glyphs = [CGGlyph](repeating: 0, count: chars.count)
	guard CTFontGetGlyphsForCharacters(font, &chars, &glyphs, chars.count), glyphs[0] != 0 else { continue }
	let v = normalize { ctx in
		ctx.setFillColor(gray: 1, alpha: 1)
		var pos = CGPoint(x: 10, y: 30)
		CTFontDrawGlyphs(font, [glyphs[0]], &pos, 1, ctx)
	}
	if let v { glyphVecs.append((cp, v)) }
}

for (n, sv) in symVecs {
	var best: (UInt32, Float) = (0, .infinity); var second: Float = .infinity
	for (cp, gv) in glyphVecs {
		var d: Float = 0
		for k in 0..<sv.count { d += abs(sv[k]-gv[k]) }
		if d < best.1 { second = best.1; best = (cp, d) } else if d < second { second = d }
	}
	print("\(n)\t\(String(best.0, radix: 16))\t\(Int(best.1))\t\(Int(second))")
}
