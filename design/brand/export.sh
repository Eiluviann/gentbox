#!/usr/bin/env bash
# Render the committed brand SVGs into design/brand/out/: PNGs, PDFs and the .icns.
# Usage: design/brand/export.sh
# Needs rsvg-convert (brew install librsvg; tested with 2.62, and older ones may drop the icon's
# feDropShadow) and iconutil (ships with macOS). Writes nothing outside out/.
set -euo pipefail
# cairo stamps each PDF with this date instead of now, so the same SVGs export byte-identical PDFs
export SOURCE_DATE_EPOCH=1767225600

here="$(cd "$(dirname "$0")" && pwd)"
svg="$here/svg"
out="$here/out"

for tool in rsvg-convert iconutil; do
	command -v "$tool" >/dev/null || { echo "export.sh: $tool not found" >&2; exit 1; }
done

rm -rf "${out:?}"
mkdir -p "$out/icon/layers" "$out/menubar" "$out/mark" "$out/wordmark"

# png <svg> <png> <width> <height>
png() {
	rsvg-convert -w "$3" -h "$4" -o "$2" "$1"
}

# pdf <svg> <pdf> <width pt> <height pt>: 72 dpi so one SVG pixel is one point
pdf() {
	rsvg-convert -f pdf -d 72 -p 72 -w "$3" -h "$4" -o "$2" "$1"
}

# App icon: an iconset per appearance (16 to 512 pt at @1x and @2x), packed into an .icns
for mode in light dark; do
	suffix=$([[ $mode == dark ]] && echo '-dark' || echo '')
	set_dir="$out/icon/Gentbox$suffix.iconset"
	mkdir -p "$set_dir"
	for size in 16 32 128 256 512; do
		png "$svg/icon-$mode.svg" "$set_dir/icon_${size}x${size}.png" "$size" "$size"
		png "$svg/icon-$mode.svg" "$set_dir/icon_${size}x${size}@2x.png" $((size * 2)) $((size * 2))
	done
	iconutil -c icns -o "$out/icon/Gentbox$suffix.icns" "$set_dir"
	png "$svg/icon-layer-background-$mode.svg" "$out/icon/layers/background-$mode.png" 1024 1024
	png "$svg/icon-layer-bubble-$mode.svg" "$out/icon/layers/bubble-$mode.png" 1024 1024
done
png "$svg/icon-layer-spark.svg" "$out/icon/layers/spark.png" 1024 1024

# Menu bar: 18 pt template images, PDF for the asset catalog and PNG at @1x and @2x
for name in idle unread urgent-body urgent-spark urgent-preview-light urgent-preview-dark; do
	pdf "$svg/menubar-$name.svg" "$out/menubar/menubar-$name.pdf" 18 18
	png "$svg/menubar-$name.svg" "$out/menubar/menubar-$name.png" 18 18
	png "$svg/menubar-$name.svg" "$out/menubar/menubar-$name@2x.png" 36 36
done

# Flat mark: square PNGs and a PDF per appearance
for mode in light dark; do
	for size in 32 64 128 256 512 1024; do
		png "$svg/mark-$mode.svg" "$out/mark/mark-$mode-$size.png" "$size" "$size"
	done
	pdf "$svg/mark-$mode.svg" "$out/mark/mark-$mode.pdf" 96 96
done

# Wordmark and lockups: PNG at 64 px tall and @2x, and a PDF, keeping each SVG's aspect ratio
for name in wordmark lockup-horizontal lockup-stacked; do
	for mode in light dark; do
		source="$svg/$name-$mode.svg"
		rsvg-convert -h 64 -o "$out/wordmark/$name-$mode.png" "$source"
		rsvg-convert -h 128 -o "$out/wordmark/$name-$mode@2x.png" "$source"
		rsvg-convert -f pdf -d 72 -p 72 -o "$out/wordmark/$name-$mode.pdf" "$source"
	done
done

echo "export.sh: wrote $(find "$out" -type f | wc -l | tr -d ' ') files to ${out#"$PWD"/}"
