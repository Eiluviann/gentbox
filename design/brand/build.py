"""Draw every Gentbox brand SVG in svg/ and examples/ from the geometry below.

Usage: python3 -I design/brand/build.py
Every mark is an original drawing on a 24-unit grid; nothing is traced from SF Symbols. The
wordmark is set in fonts/InstrumentSans-Variable.ttf at weight 600 and converted to outlines, and
the spark's well and the menu-bar cut are path booleans, so every SVG in svg/ is plain filled
paths: no font, mask or stroke. Needs fontTools and skia-pathops (pip install fonttools skia-pathops) and
hb-shape (brew install harfbuzz) for the kerning. Run it only after changing the drawing;
export.sh renders the committed SVGs.
"""
import functools, json, math, shutil, subprocess, sys
from pathlib import Path

import pathops
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.svgLib.path import parse_path
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

HERE = Path(__file__).resolve().parent
FONT = HERE / 'fonts' / 'InstrumentSans-Variable.ttf'
WORD, WEIGHT, TRACKING = 'gentbox', 600, -0.02  # tracking in em, as CSS letter-spacing

# The 24-unit grid: a radius-3 bubble with a squared tab tail, and a four-point spark
SW = 1.75  # menu-bar outline stroke: 1.31 pt at 18 pt
BODY = dict(x0=3, y0=3.5, x1=21, y1=17, r=3, tail_x=7.5, tail_w=4, tail_y=20.75)
ICON_SPARK = (12, 10.25, 3.4, 3.4)  # centre x, centre y, half height, half width
ICON_WELL = 1.3  # the knocked-out ring around the icon spark: 0.65 each side
GLYPH_SPARK = (18.25, 6.25, 4, 4)  # the menu-bar spark lands on the top-right corner
GLYPH_CUT = 3.2  # the outline stops 1.6 short of the menu-bar spark
URGENT = {'light': '#FF3B30', 'dark': '#FF453A'}  # systemRed; the app draws it with NSColor.systemRed
CONIC_TOLERANCE = 0.002  # grid units: skia's default 0.25 flattens the menu-bar corners and thins the stroke
SLIVER = 0.5  # pieces of the cut outline smaller than this, in square units, are dropped

# Icon tile and art per appearance; FLAT is the one-colour-plus-spark mark for pages and documents
TILE = {
	'light': dict(top='#26385F', bottom='#121C33', paper='#F6F4EF', amber='#FFB224'),
	'dark': dict(top='#2C2C2F', bottom='#141416', paper='#E9E7E2', amber='#FFB224'),
}
FLAT = {
	'light': dict(ink='#1B2A4A', spark='#E89600'),
	'dark': dict(ink='#F2F0EB', spark='#FFB224'),
}
PAGE = {'light': '#FFFFFF', 'dark': '#1E1E1E'}

# Every SVG carries a <title> for screen readers; a -light or -dark suffix adds the appearance
TITLES = {
	'icon': 'Gentbox app icon',
	'icon-layer-background': 'Gentbox app icon background layer',
	'icon-layer-bubble': 'Gentbox app icon bubble layer',
	'icon-layer-spark': 'Gentbox app icon spark layer',
	'mark': 'Gentbox mark',
	'menubar-idle': 'Gentbox menu-bar glyph, nothing waiting',
	'menubar-unread': 'Gentbox menu-bar glyph, unread',
	'menubar-urgent-body': 'Gentbox menu-bar glyph, urgent body',
	'menubar-urgent-spark': 'Gentbox menu-bar glyph, urgent spark',
	'menubar-urgent-preview': 'Gentbox menu-bar glyph, urgent',
	'wordmark': 'Gentbox wordmark',
	'lockup-horizontal': 'Gentbox horizontal lockup',
	'lockup-stacked': 'Gentbox stacked lockup',
	'clear-space-lockup': 'Clear space around the horizontal lockup',
	'do-clear-space': 'Mark with its clear space marked',
	'do-dark': 'Dark mark on a dark page',
	'dont-recolour': 'Recoloured mark',
	'dont-stretch': 'Stretched mark',
	'dont-rotate': 'Rotated mark',
	'dont-fill-well': 'Mark with the well filled',
	'dont-glyph-as-logo': 'Menu-bar glyph used as a logo',
	'dont-effects': 'Mark with a drop shadow',
}

# The legacy icon sits on Apple's 824 px tile inside a 1024 canvas; Icon Composer layers are full bleed
LEGACY_SCALE, LEGACY_OFFSET = 26, 200
LAYER_SCALE = LEGACY_SCALE * 1024 / 824
LAYER_OFFSET = 512 - 12 * LAYER_SCALE


def main():
	files = {}
	for mode in ('light', 'dark'):
		files |= {
			f'svg/icon-{mode}.svg': icon(mode),
			f'svg/icon-layer-background-{mode}.svg': layer_background(mode),
			f'svg/icon-layer-bubble-{mode}.svg': layer_bubble(mode),
			f'svg/mark-{mode}.svg': flat_mark(mode),
			f'svg/menubar-urgent-preview-{mode}.svg': menubar_urgent_preview(mode),
		}
	files |= {
		'svg/icon-layer-spark.svg': layer_spark(),
		'svg/menubar-idle.svg': menubar_idle(),
		'svg/menubar-unread.svg': menubar_unread(),
		'svg/menubar-urgent-body.svg': menubar_urgent_body(),
		'svg/menubar-urgent-spark.svg': menubar_urgent_spark(),
	}
	word = load_wordmark()
	for mode in ('light', 'dark'):
		files |= {
			f'svg/wordmark-{mode}.svg': wordmark(word, mode),
			f'svg/lockup-horizontal-{mode}.svg': lockup_horizontal(word, mode),
			f'svg/lockup-stacked-{mode}.svg': lockup_stacked(word, mode),
		}
	files |= examples(word)
	for name, text in files.items():
		path = HERE / name
		path.parent.mkdir(exist_ok=True)
		path.write_text(titled(text, path.stem))
	print(f'wrote {len(files)} SVGs')


# App icon

def icon(mode):
	"""The full icon on Apple's macOS grid: gradient tile, paper bubble, amber spark in its well."""
	t = TILE[mode]
	body = (
		f'<defs>{gradient("tile", t)}'
		'<filter id="tile-shadow" x="-10%" y="-10%" width="120%" height="125%"><feDropShadow dx="0" dy="10" stdDeviation="12" flood-color="#000" flood-opacity="0.28"/></filter>'
		'<filter id="art-shadow" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="0.45" stdDeviation="0.5" flood-color="#000" flood-opacity="0.3"/></filter>'
		'</defs>'
		f'<path d="{squircle()}" fill="url(#tile)" filter="url(#tile-shadow)"/>'
		f'<g transform="translate({LEGACY_OFFSET} {LEGACY_OFFSET}) scale({LEGACY_SCALE})" filter="url(#art-shadow)">'
		f'<path d="{bubble_with_well()}" fill="{t["paper"]}"/><path d="{spark(*ICON_SPARK)}" fill="{t["amber"]}"/></g>'
	)
	return svg(1024, 1024, body)


def layer_background(mode):
	"""Full-bleed tile gradient for Icon Composer, which applies the shape and the glass itself."""
	return svg(1024, 1024, f'<defs>{gradient("tile", TILE[mode])}</defs><rect width="1024" height="1024" fill="url(#tile)"/>')


def layer_bubble(mode):
	return svg(1024, 1024, f'<g transform="{layer_transform()}"><path d="{bubble_with_well()}" fill="{TILE[mode]["paper"]}"/></g>')


def layer_spark():
	return svg(1024, 1024, f'<g transform="{layer_transform()}"><path d="{spark(*ICON_SPARK)}" fill="{TILE["light"]["amber"]}"/></g>')


def flat_mark(mode, fill=None):
	"""The mark for pages and documents: ink bubble, spark in its well, no tile."""
	return svg(96, 96, mark_art(mode, fill), vb='1.5 1.5 21 21')


# Menu-bar glyphs: 18 pt template images on the 24 grid

def menubar_idle():
	return svg(24, 24, f'<path d="{glyph_idle()}"/>')


def menubar_unread():
	return svg(24, 24, f'<path d="{glyph_cut()}"/><path d="{spark(*GLYPH_SPARK)}"/>')


def menubar_urgent_body():
	"""The unread outline without its spark: a template image the app tints, under a red spark."""
	return svg(24, 24, f'<path d="{glyph_cut()}"/>')


def menubar_urgent_spark():
	return svg(24, 24, f'<path d="{spark(*GLYPH_SPARK)}" fill="{URGENT["light"]}"/>')


def menubar_urgent_preview(mode):
	ink = '#000' if mode == 'light' else '#FFF'
	return svg(24, 24, f'<path d="{glyph_cut()}" fill="{ink}"/><path d="{spark(*GLYPH_SPARK)}" fill="{URGENT[mode]}"/>')


# Wordmark and lockups

def load_wordmark():
	"""Outline the word at WEIGHT with the font's own kerning, in font units with y pointing down."""
	if shutil.which('hb-shape') is None:
		sys.exit('build.py: hb-shape not found; brew install harfbuzz')
	font = instancer.instantiateVariableFont(TTFont(FONT), {'wght': WEIGHT, 'wdth': 100})
	upem = font['head'].unitsPerEm
	glyphs = font.getGlyphSet()
	shaped = json.loads(subprocess.run(
		['hb-shape', f'--variations=wght={WEIGHT}', '--output-format=json', str(FONT), WORD],
		check=True, capture_output=True, text=True,
	).stdout)
	parts, x, top, bottom = [], 0.0, 0.0, 0.0
	for i, item in enumerate(shaped):
		pen, bounds = SVGPathPen(glyphs), BoundsPen(glyphs)
		glyphs[item['g']].draw(pen)
		glyphs[item['g']].draw(bounds)
		parts.append(f'<path transform="matrix(1 0 0 -1 {fmt(x + item["dx"])} 0)" d="{pen.getCommands()}"/>')
		top, bottom = max(top, bounds.bounds[3]), min(bottom, bounds.bounds[1])
		x += item['ax'] + (TRACKING * upem if i < len(shaped) - 1 else 0)
	return dict(
		paths=''.join(parts), width=x, top=-top, bottom=-bottom, upem=upem,
		x_height=font['OS/2'].sxHeight,
	)


def wordmark(word, mode, fill=None):
	height = word['bottom'] - word['top']
	scale = 48 / height
	return svg(
		round(word['width'] * scale, 1), 48,
		f'<g fill="{fill or FLAT[mode]["ink"]}">{word["paths"]}</g>',
		vb=f'0 {fmt(word["top"])} {fmt(word["width"])} {fmt(height)}',
	)


def lockup_horizontal(word, mode):
	"""Mark and word side by side, as on the review boards: the mark box is 1.25 em, the gap 0.3 em,
	and the word's x-height is centred on the spark."""
	em = 21 / 1.25
	scale = em / word['upem']
	x = 22.5 + 0.3 * em
	baseline = ICON_SPARK[1] + word['x_height'] * scale / 2
	width = x + word['width'] * scale + 1.5
	text = f'<g transform="translate({fmt(x)} {fmt(baseline)}) scale({fmt(scale, 6)})" fill="{FLAT[mode]["ink"]}">{word["paths"]}</g>'
	return svg(round((width - 1.5) * 8), 168, mark_art(mode) + text, vb=f'1.5 1.5 {fmt(width - 1.5)} 21')


def lockup_stacked(word, mode):
	"""Mark over the word, centred, with the word set at 0.4 of the mark box."""
	scale = 8.4 / word['upem']
	text_width = word['width'] * scale
	width = max(21, text_width + 3)
	mark_x = (width - 21) / 2
	baseline = 21 + 3 - word['top'] * scale
	height = baseline + word['bottom'] * scale + 1.5
	body = (
		f'<g transform="translate({fmt(mark_x - 1.5)} -1.5)">{mark_art(mode)}</g>'
		f'<g transform="translate({fmt((width - text_width) / 2)} {fmt(baseline)}) scale({fmt(scale, 6)})" fill="{FLAT[mode]["ink"]}">{word["paths"]}</g>'
	)
	return svg(round(width * 8), round(height * 8), body, vb=f'0 0 {fmt(width)} {fmt(height)}')


# Do and don't examples for the README

def examples(word):
	ink, spark_fill = FLAT['light']['ink'], FLAT['light']['spark']
	clear = spark_height = 2 * ICON_SPARK[2]
	tile = lambda content, w=40, h=30, vb=None: svg(w * 6, h * 6, f'<rect x="-100" y="-100" width="400" height="400" fill="#FFFFFF"/>{content}', vb=vb or f'0 0 {w} {h}')
	mark = lambda attrs='', fill=None: f'<g {attrs}>{mark_art("light", fill)}</g>'
	space = (
		f'<rect x="{1.5 - clear}" y="{1.5 - clear}" width="{21 + 2 * clear}" height="{21 + 2 * clear}" fill="none" stroke="#0A84FF" stroke-width="0.25" stroke-dasharray="0.8 0.6"/>'
		f'<rect x="1.5" y="1.5" width="21" height="21" fill="none" stroke="#0A84FF" stroke-opacity="0.4" stroke-width="0.15"/>'
	)
	no_well = f'<path d="{bubble()}" fill="{ink}"/><path d="{spark(*ICON_SPARK)}" fill="{spark_fill}"/>'
	return {
		'examples/do-clear-space.svg': tile(mark() + space, vb=f'{1.5 - clear - 2} {1.5 - clear - 2} {21 + 2 * clear + 4} {21 + 2 * clear + 4}', w=round(21 + 2 * clear + 4), h=round(21 + 2 * clear + 4)),
		'examples/do-dark.svg': svg(240, 180, f'<rect x="-100" y="-100" width="400" height="400" fill="{PAGE["dark"]}"/>{mark_art("dark")}', vb='-8 -3 40 30'),
		'examples/dont-recolour.svg': tile(mark('transform="translate(8 3)"', fill=dict(ink='#2E9E4F', spark='#7D7AFF'))),
		'examples/dont-stretch.svg': tile(mark('transform="translate(-4 3) scale(1.6 1)"')),
		'examples/dont-rotate.svg': tile(mark('transform="translate(8 3) rotate(-18 12 12)"')),
		'examples/dont-fill-well.svg': tile(f'<g transform="translate(8 3)">{no_well}</g>'),
		'examples/dont-glyph-as-logo.svg': tile(f'<g transform="translate(8 3)" fill="{ink}"><path d="{glyph_cut()}"/><path d="{spark(*GLYPH_SPARK)}"/></g>'),
		'examples/dont-effects.svg': tile(
			'<defs><filter id="glow" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="0.8" dy="1.2" stdDeviation="0.9" flood-color="#000" flood-opacity="0.55"/></filter></defs>'
			+ mark('transform="translate(8 3)" filter="url(#glow)"')
		),
	} | {'examples/clear-space-lockup.svg': clear_space_lockup(word, clear)}


def clear_space_lockup(word, clear):
	inner = lockup_horizontal(word, 'light').split('>', 1)[1].rsplit('</svg>', 1)[0]
	em = 21 / 1.25
	width = 22.5 + 0.3 * em + word['width'] * em / word['upem'] + 1.5
	box = (
		f'<rect x="{fmt(1.5 - clear)}" y="{fmt(1.5 - clear)}" width="{fmt(width - 1.5 + 2 * clear)}" height="{fmt(21 + 2 * clear)}" fill="none" stroke="#0A84FF" stroke-width="0.25" stroke-dasharray="0.8 0.6"/>'
		f'<path d="{spark(1.5 - clear / 2, 12, ICON_SPARK[2], ICON_SPARK[3])}" fill="#0A84FF" fill-opacity="0.35"/>'
	)
	x0, y0 = 1.5 - clear - 2, 1.5 - clear - 2
	w, h = width - 1.5 + 2 * clear + 4, 21 + 2 * clear + 4
	return svg(round(w * 6), round(h * 6), f'<rect x="{fmt(x0)}" y="{fmt(y0)}" width="{fmt(w)}" height="{fmt(h)}" fill="#FFFFFF"/>{inner}{box}', vb=f'{fmt(x0)} {fmt(y0)} {fmt(w)} {fmt(h)}')


# Shared pieces

def mark_art(mode, fill=None):
	colours = fill or FLAT[mode]
	return f'<path d="{bubble_with_well()}" fill="{colours["ink"]}"/><path d="{spark(*ICON_SPARK)}" fill="{colours["spark"]}"/>'


@functools.cache
def bubble_with_well():
	"""The filled bubble less the spark and a ring of ICON_WELL / 2 around it."""
	return path_d(pathops.op(path_of(bubble()), grown(spark(*ICON_SPARK), ICON_WELL), pathops.PathOp.DIFFERENCE))


@functools.cache
def glyph_idle():
	return path_d(stroked(bubble(), SW))


@functools.cache
def glyph_cut():
	"""The menu-bar outline, stopping GLYPH_CUT / 2 short of the corner spark. The grown spark is
	concave between its tips, so the cut leaves a sliver of the corner arc; drop it."""
	cut = pathops.op(stroked(bubble(), SW), grown(spark(*GLYPH_SPARK), GLYPH_CUT), pathops.PathOp.DIFFERENCE)
	kept = pathops.Path()
	for contour in cut.contours:
		if abs(contour.area) >= SLIVER:
			kept.addPath(contour)
	assert len(list(kept.contours)) == 1, 'the menu-bar cut must leave a single outline'
	return path_d(kept)


def grown(d, width):
	"""A closed shape plus a ring of width / 2 around it, with round corners."""
	return pathops.op(path_of(d), stroked(d, width), pathops.PathOp.UNION)


def stroked(d, width):
	path = path_of(d)
	path.stroke(width, pathops.LineCap.BUTT_CAP, pathops.LineJoin.ROUND_JOIN, 4)
	path.convertConicsToQuads(CONIC_TOLERANCE)
	return pathops.simplify(path)


def path_of(d):
	path = pathops.Path()
	parse_path(d, path.getPen())
	return path


def path_d(path):
	pen = SVGPathPen(None, ntos=fmt)
	path.draw(pen)
	return pen.getCommands()


def gradient(name, tile):
	return f'<linearGradient id="{name}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{tile["top"]}"/><stop offset="1" stop-color="{tile["bottom"]}"/></linearGradient>'


def layer_transform():
	return f'translate({fmt(LAYER_OFFSET)} {fmt(LAYER_OFFSET)}) scale({fmt(LAYER_SCALE)})'


def bubble():
	"""Bubble body with a squared tab tail whose left edge drops straight down at tail_x."""
	b = BODY
	x0, y0, x1, y1, r = b['x0'], b['y0'], b['x1'], b['y1'], b['r']
	return (
		f'M{x0 + r} {y0}H{x1 - r}A{r} {r} 0 0 1 {x1} {y0 + r}V{y1 - r}A{r} {r} 0 0 1 {x1 - r} {y1}'
		f'H{b["tail_x"] + b["tail_w"]}L{b["tail_x"]} {b["tail_y"]}V{y1}H{x0 + r}A{r} {r} 0 0 1 {x0} {y1 - r}V{y0 + r}A{r} {r} 0 0 1 {x0 + r} {y0}Z'
	)


def spark(cx, cy, a, b, w=0.2):
	"""Four-point spark with a round waist (w) rather than a pinched one."""
	return (
		f'M{fmt(cx)} {fmt(cy - a)}Q{fmt(cx + w * b)} {fmt(cy - w * a)} {fmt(cx + b)} {fmt(cy)}'
		f'Q{fmt(cx + w * b)} {fmt(cy + w * a)} {fmt(cx)} {fmt(cy + a)}'
		f'Q{fmt(cx - w * b)} {fmt(cy + w * a)} {fmt(cx - b)} {fmt(cy)}'
		f'Q{fmt(cx - w * b)} {fmt(cy - w * a)} {fmt(cx)} {fmt(cy - a)}Z'
	)


def squircle(cx=512, cy=512, r=412, n=5.0, steps=240):
	points = []
	for i in range(steps):
		t = 2 * math.pi * i / steps
		c, s = math.cos(t), math.sin(t)
		points.append(f'{cx + r * math.copysign(abs(c) ** (2 / n), c):.1f} {cy + r * math.copysign(abs(s) ** (2 / n), s):.1f}')
	return 'M' + 'L'.join(points) + 'Z'


def titled(text, stem):
	base, _, mode = stem.rpartition('-')
	title = TITLES.get(stem) or f'{TITLES[base]}, {mode}'
	return text.replace('>', f'><title>{title}</title>', 1)


def svg(w, h, body, vb=None):
	return f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{vb or f"0 0 {w} {h}"}">{body}</svg>\n'


def fmt(value, places=3):
	return f'{value:.{places}f}'.rstrip('0').rstrip('.')


if __name__ == '__main__':
	sys.exit(main())
