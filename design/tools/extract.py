import json, sys
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.boundsPen import BoundsPen

font = instancer.instantiateVariableFont(TTFont('/Library/Fonts/SF-Pro.ttf'), {'wght': 400, 'opsz': 17, 'wdth': 100})
gs = font.getGlyphSet()
cmap = font.getBestCmap()

def bounds(name):
	bp = BoundsPen(gs); gs[name].draw(bp); return bp.bounds

ref = bounds(cmap[0x100000])  # circle: the size floor so small glyphs keep their scale
refSize = max(ref[2] - ref[0], ref[3] - ref[1])
out = {}
for line in open('map.tsv'):
	name, cp, *_ = line.split('\t')
	g = cmap[int(cp, 16)]
	x0, y0, x1, y1 = bounds(g)
	size = max(x1 - x0, y1 - y0, refSize) * 1.0
	cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
	pen = SVGPathPen(gs, ntos=lambda v: ('%.1f' % v).rstrip('0').rstrip('.'))
	# flip y and move the square frame to the origin, scaled to a 100-unit box
	k = 100 / size
	gs[g].draw(TransformPen(pen, (k, 0, 0, -k, -(cx - size / 2) * k, (cy + size / 2) * k)))
	out[name] = pen.getCommands()
json.dump(out, open('icons.json', 'w'), indent='\t')
# contact sheet
cells = []
for i, (n, d) in enumerate(out.items()):
	x, y = (i % 10) * 120, (i // 10) * 120
	cells.append(f'<g transform="translate({x+30},{y+10}) scale(0.6)"><path d="{d}"/></g><text x="{x+60}" y="{y+100}" font-size="11" text-anchor="middle" font-family="Helvetica">{n[:20]}</text>')
rows = (len(out) + 9) // 10
open('sheet.svg', 'w').write(f'<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="{rows*120}"><rect width="100%" height="100%" fill="#fff"/>{"".join(cells)}</svg>')
print(len(out), sum(len(v) for v in out.values()))
