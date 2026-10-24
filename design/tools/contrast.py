"""Check WCAG contrast for every text and icon pair in project/an.css, light and dark.

Usage: python3 -I design/tools/contrast.py [--markdown]
Reads the light-dark(light, dark) tokens, lays translucent colours over the surface
they sit on, and fails (exit 1) when a pair is under its minimum: 4.5:1 for text,
3:1 for icons and other non-text marks. A pair in ACCEPTED, which the user accepted
below its minimum, prints as accepted and does not fail.
"""
import re, sys
from pathlib import Path

CSS = Path(__file__).resolve().parent.parent / 'project' / 'an.css'

# (foreground, background, under, minimum, where[, opacity]): `under` is the surface a translucent
# background sits on; `opacity` fades the foreground, as CSS opacity on the text or icon would
TEXT, ICON = 4.5, 3.0
# Pairs under their minimum that the user accepted, with the reason; they print as accepted, not failing
ACCEPTED = {
	('on-fill', 'accent'): 'system accent fill (.borderedProminent), for consistency with macOS; Increase Contrast darkens it',
}
PAIRS = [
	('ink', 'surface', None, TEXT, 'Body text in the main window'),
	('ink', 'elevated', None, TEXT, 'Body text in the popover, popups and toasts'),
	('ink', 'sidebar', None, TEXT, 'Sidebar items'),
	('ink', 'card', None, TEXT, 'Settings tiles'),
	('ink', 'menu', 'elevated', TEXT, 'Menu items'),
	('ink', 'banner', 'desk', TEXT, 'Native banner text'),
	('ink', 'control', None, TEXT, 'Button labels'),
	('ink', 'field', None, TEXT, 'Text fields'),
	('ink', 'seg-on', 'track', TEXT, 'Selected segment'),
	('ink', 'track', 'elevated', TEXT, 'Code boxes and grey tags'),
	('ink-2', 'surface', None, TEXT, 'Secondary text in the window'),
	('ink-2', 'elevated', None, TEXT, 'Secondary text in the popover'),
	('ink-2', 'card', None, TEXT, 'Secondary text on tiles'),
	('ink-2', 'track', 'elevated', TEXT, 'Tag text on a track'),
	('ink-2', 'wait-soft', None, TEXT, 'Body text on a red callout'),
	('ink-2', 'warn-soft', None, TEXT, 'Body text on a warning callout'),
	('ink-3', 'surface', None, TEXT, 'Captions and breadcrumbs in the window'),
	('ink-3', 'elevated', None, TEXT, 'Captions and breadcrumbs in the popover'),
	('ink-3', 'sidebar', None, TEXT, 'Sidebar headings'),
	('ink-3', 'card', None, TEXT, 'Captions on tiles'),
	('ink-3', 'track', 'elevated', TEXT, 'Agent working and Withdrawn tags'),
	('ink-3', 'track', 'surface', TEXT, 'Grey tags in the window'),
	('ink-3', 'menu', 'elevated', TEXT, 'Disabled menu items'),
	('accent-ink', 'surface', None, TEXT, 'Links in the window'),
	('accent-ink', 'elevated', None, TEXT, 'Links in the popover'),
	('on-fill', 'accent', None, TEXT, 'Primary button, selected menu item, chips'),
	('danger-ink', 'control', None, TEXT, 'Deny button label'),
	('on-fill', 'badge', None, TEXT, 'Count badge'),
	('dec-ink', 'dec-soft', None, TEXT, 'Decision tag'),
	('act-ink', 'act-soft', None, TEXT, 'Action tag'),
	('done-ink', 'done-soft', None, TEXT, 'Completed tag'),
	('info-ink', 'info-soft', None, TEXT, 'Generic tag'),
	('wait-ink', 'wait-soft', None, TEXT, 'Agent waiting tag'),
	('paused-ink', 'paused-soft', None, TEXT, 'Paused · can defer tag'),
	('wait-ink', 'elevated', None, TEXT, 'Urgent and waiting words on the popover'),
	('wait-ink', 'surface', None, TEXT, 'Urgent and waiting words in the window'),
	('paused-ink', 'elevated', None, TEXT, 'Paused words on the popover'),
	('done-ink', 'elevated', None, TEXT, 'Done words on the popover'),
	('act-ink', 'surface', None, TEXT, 'Done on Mac in the window'),
	('menubar-ink', 'menubar', 'desk', TEXT, 'Menu-bar clock'),
	('ink-3', 'control', None, TEXT, 'Shortcut hint on a bordered button'),
	('on-fill', 'accent', None, TEXT, 'Shortcut hint on a primary button'),
	('ink', 'sel', 'sidebar', TEXT, 'Selected sidebar item'),
	('ink', 'accent-soft', 'elevated', TEXT, 'Row picked to dismiss'),
	('ink-3', 'accent-soft', 'elevated', TEXT, 'Breadcrumb on a picked row'),
	('dec', 'elevated', None, ICON, 'Decision symbol'),
	('act', 'elevated', None, ICON, 'Action symbol'),
	('done', 'elevated', None, ICON, 'Completed symbol'),
	('info', 'elevated', None, ICON, 'Generic symbol'),
	('wait', 'elevated', None, ICON, 'Waiting pause and urgent figure'),
	('paused', 'elevated', None, ICON, 'Paused pause symbol'),
	('dec', 'surface', None, ICON, 'Decision symbol in the window'),
	('act', 'surface', None, ICON, 'Action symbol in the window'),
	('done', 'surface', None, ICON, 'Completed symbol in the window'),
	('wait', 'surface', None, ICON, 'Waiting symbol in the window'),
	('paused', 'surface', None, ICON, 'Paused symbol in the window'),
	('accent', 'sidebar', None, ICON, 'Sidebar symbols'),
	('accent', 'elevated', None, ICON, 'Toggle on, progress bar'),
	('danger', 'control', None, ICON, 'Deny symbol'),
	('topic-release', 'elevated', None, ICON, 'Topic symbol'),
	('topic-deps', 'sidebar', None, ICON, 'Topic symbol in the sidebar'),
	('warn-ink', 'warn-soft', None, ICON, 'Warning symbol'),
	('wait', 'menubar', 'desk', ICON, 'Menu-bar red dot'),
	('accent', 'sel', 'sidebar', ICON, 'Symbol on the selected sidebar item'),
	('topic-release', 'sel', 'sidebar', ICON, 'Topic symbol on the selected sidebar item'),
	('topic-deps', 'sel', 'sidebar', ICON, 'Topic symbol on the selected sidebar item'),
	('dec', 'accent-soft', 'elevated', ICON, 'Decision symbol on a picked row'),
	('done', 'accent-soft', 'elevated', ICON, 'Completed symbol on a picked row'),
	('paused', 'accent-soft', 'elevated', ICON, 'Paused symbol on a picked row'),
	('wait', 'accent-soft', 'elevated', ICON, 'Waiting symbol on a picked row'),
	('accent', 'accent-soft', 'elevated', ICON, 'Selection circle on a picked row'),
]


def main():
	tokens = read_tokens(CSS.read_text())
	rows, failures = [], 0
	for fg, bg, under, minimum, where, *faded in PAIRS:
		opacity = faded[0] if faded else 1.0
		ratios = [ratio_for(tokens, fg, bg, under, mode, opacity) for mode in (0, 1)]
		passes = all(r >= minimum for r in ratios)
		status = 'ok' if passes else 'accepted' if (fg, bg) in ACCEPTED else 'FAIL'
		failures += status == 'FAIL'
		rows.append((where, fg, bg, minimum, opacity, ratios, status))
	if '--markdown' in sys.argv:
		print_markdown(rows)
	else:
		for where, fg, bg, minimum, opacity, (light, dark), status in rows:
			print(f"{status:<8} {light:5.2f} {dark:5.2f}  >= {minimum}  {fg} at {opacity:g} on {bg}  ({where})")
	sys.exit(1 if failures else 0)


def print_markdown(rows):
	print('| Pair | Where | Opacity | Minimum | Light | Dark | Status |')
	print('| --- | --- | --- | --- | --- | --- | --- |')
	for where, fg, bg, minimum, opacity, (light, dark), status in rows:
		note = f'accepted: {ACCEPTED[(fg, bg)]}' if status == 'accepted' else status
		print(f'| `{fg}` on `{bg}` | {where} | {opacity:.0%} | {minimum:g}:1 | {light:.2f} | {dark:.2f} | {note} |')


def ratio_for(tokens, fg, bg, under, mode, opacity):
	base = resolve(tokens, under or 'board', mode)
	if base[3] < 1:
		base = over(base, resolve(tokens, 'board', mode))
	back = over(resolve(tokens, bg, mode), base)
	colour = resolve(tokens, fg, mode)
	front = over(colour[:3] + (colour[3] * opacity,), back)
	return contrast(front, back)


def read_tokens(css):
	tokens = {}
	for name, value in re.findall(r'--([a-z0-9-]+):\s*([^;]+);', css):
		tokens.setdefault(name, ' '.join(value.split()))
	return tokens


def resolve(tokens, name, mode):
	value = tokens[name]
	alias = re.fullmatch(r'var\(--([a-z0-9-]+)\)', value)
	if alias:
		return resolve(tokens, alias.group(1), mode)
	pair = re.fullmatch(r'light-dark\((.*)\)', value)
	if pair:
		value = split_top_level(pair.group(1))[mode]
	return parse_colour(value)


def split_top_level(text):
	parts, depth, start = [], 0, 0
	for i, ch in enumerate(text):
		depth += ch == '('
		depth -= ch == ')'
		if ch == ',' and depth == 0:
			parts.append(text[start:i].strip())
			start = i + 1
	parts.append(text[start:].strip())
	return parts


def parse_colour(value):
	value = value.strip()
	if value.startswith('#'):
		hexes = value[1:]
		if len(hexes) == 3:
			hexes = ''.join(c * 2 for c in hexes)
		return (int(hexes[0:2], 16), int(hexes[2:4], 16), int(hexes[4:6], 16), 1.0)
	numbers = [float(n) for n in re.findall(r'[\d.]+', value)]
	return (numbers[0], numbers[1], numbers[2], numbers[3] if len(numbers) > 3 else 1.0)


def over(top, bottom):
	alpha = top[3]
	return tuple(top[i] * alpha + bottom[i] * (1 - alpha) for i in range(3)) + (1.0,)


def luminance(colour):
	def channel(c):
		c /= 255
		return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
	r, g, b = (channel(c) for c in colour[:3])
	return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a, b):
	la, lb = sorted((luminance(a), luminance(b)), reverse=True)
	return (la + 0.05) / (lb + 0.05)


main()
