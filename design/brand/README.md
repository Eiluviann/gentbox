# Gentbox brand kit

The Gentbox mark is a squared speech bubble with an amber spark: a message from an agent that needs you. The app icon and the logo carry the spark inside the bubble; the menu bar carries an outline bubble whose spark lands on its top-right corner when something is unread.

Every mark here is an original drawing on a 24-unit grid. None is traced from or built on an SF Symbol: Apple's SF Symbols licence forbids symbols or their derivatives in app icons, logos and trademarks. SF Symbols stay in the UI mockups only.

## Layout

| Path | What |
| --- | --- |
| `build.py` | Draws every SVG in `svg/` and `examples/` from the geometry at its top |
| `svg/` | Committed sources: app icon, Icon Composer layers, flat mark, menu-bar glyphs, wordmark, lockups |
| `examples/` | The do and don't images below |
| `export.sh` | Renders `svg/` into `out/` (ignored by git): PNGs, PDFs and the `.icns` |
| `brand.css` | Brand colour tokens, in the `light-dark()` form of `../project/an.css` |
| `fonts/` | Instrument Sans, the wordmark typeface, and its licence |

The art in `svg/` is filled paths: the wordmark is outlined, and the spark's well and the menu-bar cut are path booleans, so there is no font, mask or stroke to render differently between tools, and the PDFs stay vector. The exceptions are the app icon's tile gradient and its two drop shadows; Icon Composer adds its own, so the layers carry none.

## Build and export

Export from a checkout, with `rsvg-convert` (`brew install librsvg`) and `iconutil` (ships with macOS):

```bash
design/brand/export.sh
```

It replaces `out/` with:

- `icon/Gentbox.icns` and `icon/Gentbox-dark.icns`, from iconsets at 16 to 512 pt, @1x and @2x
- `icon/layers/`: the three Icon Composer layers at 1024 px
- `menubar/`: each glyph as an 18 pt PDF and an 18 and 36 px PNG
- `mark/`: the flat mark from 32 to 1024 px, and a PDF
- `wordmark/`: the wordmark and both lockups at 64 px tall, @2x, and a PDF

Redraw the SVGs only after changing the geometry. `build.py` needs fontTools, skia-pathops and `hb-shape`; the committed SVGs were drawn with fontTools 4.67, skia-pathops 0.9 and HarfBuzz 14.2, and other versions may move the outlines slightly:

```bash
brew install harfbuzz
```

```bash
python3 -m pip install fonttools skia-pathops
```

```bash
python3 -I design/brand/build.py
```

## App icon

`icon-light.svg` and `icon-dark.svg` are the finished icon: a paper bubble on a deep navy tile (graphite in dark mode), with the amber spark sitting in a well knocked out of the bubble, so the tile shows around it. They feed the legacy `.icns`.

For Icon Composer (macOS 26 and later), import the full-bleed layers from `out/icon/layers/` bottom to top: `background-<mode>`, `bubble-<mode>`, then `spark`. Icon Composer adds the shape, the glass and the shadows, so the layers carry none.

## Menu-bar glyph

An 18 pt template image with a 1.31 pt outline. Add `menubar-idle.pdf`, `menubar-unread.pdf` and `menubar-urgent-body.pdf` to the asset catalog with Render As set to Template Image, so macOS tints them for the menu bar. `menubar-urgent-spark.pdf` is only the spark's shape: add it as a template too and draw it tinted with `NSColor.systemRed`.

| State | Asset | How it draws |
| --- | --- | --- |
| Nothing waiting | `menubar-idle` | Outline bubble, template |
| Unread | `menubar-unread` | The outline stops short of a spark on the top-right corner; template |
| Urgent | `menubar-urgent-body` and `menubar-urgent-spark` | Both are templates; the app draws the spark over the body tinted with `NSColor.systemRed`, since a template image holds no colour |

`menubar-urgent-preview-light` and `-dark` show the composed urgent state for documents and boards only; leave them out of the asset catalog. The quiet-hours (Focus) state and the waiting count are not drawn yet.

## Colours

`brand.css` holds the brand tokens; `design/tools/contrast.py` checks them with the rest of the design tokens.

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `--brand-ink` | `#1b2a4a` | `#f2f0eb` | Flat mark bubble, wordmark |
| `--brand-spark` | `#e89600` | `#ffb224` | Flat mark spark |
| `--brand-tile-top` | `#26385f` | `#2c2c2f` | App icon gradient, top |
| `--brand-tile-bottom` | `#121c33` | `#141416` | App icon gradient, bottom |
| `--brand-paper` | `#f6f4ef` | `#e9e7e2` | App icon bubble |
| `--brand-amber` | `#ffb224` | `#ffb224` | App icon spark |
| `--brand-urgent` | `#ff3b30` | `#ff453a` | Urgent menu-bar spark (`systemRed` in the app) |

The light flat spark, `#e89600` on white, is 2.39:1. Logos are exempt from WCAG's non-text contrast rule, and the navy bubble carries the mark on its own, so the spark stays the warmer amber rather than darkening to brown; `contrast.py` lists it as accepted. Don't use `--brand-spark` for text or for an icon that has to be read on its own.

## Typeface

The wordmark is Instrument Sans SemiBold (weight 600, width 100) with -0.02em tracking, lowercase, outlined in `svg/`. Instrument Sans is under the SIL Open Font License 1.1 (`fonts/OFL.txt`), so it can be bundled and the outlines shipped. Set any other brand text in the system font, as the app does.

## Clear space and minimum size

Clear space is X on every side of the mark's square box, where X is the spark's height (about half the bubble's height). Keep text, edges and other marks out of it.

![Clear space around the horizontal lockup](examples/clear-space-lockup.svg)

| Asset | Smallest |
| --- | --- |
| Flat mark | 24 px; below that use the app icon |
| Horizontal lockup | Mark at 24 px tall |
| Stacked lockup | Mark at 32 px tall |
| Wordmark alone | 14 px tall |
| Menu-bar glyph | 18 pt only; it is drawn for that size |

## Do and don't

| Do | Don't |
| --- | --- |
| ![Mark with its clear space marked](examples/do-clear-space.svg) Keep clear space around the mark | ![Recoloured mark](examples/dont-recolour.svg) Recolour the bubble or the spark |
| ![Dark mark on a dark page](examples/do-dark.svg) Use the dark version on dark backgrounds | ![Stretched mark](examples/dont-stretch.svg) Stretch or squash the mark |
| | ![Rotated mark](examples/dont-rotate.svg) Rotate the mark |
| | ![Mark with the well filled](examples/dont-fill-well.svg) Fill in the well around the spark |
| | ![Menu-bar glyph used as a logo](examples/dont-glyph-as-logo.svg) Use the menu-bar glyph as the logo |
| | ![Mark with a drop shadow](examples/dont-effects.svg) Add shadows, glows or other effects |
