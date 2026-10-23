"""Render .dc.html boards as static pages for a local look (holes filled from sf-symbols.js).

Usage: python3 -I design/tools/preview.py <out-dir> [board.dc.html ...]
Only resolves {{ic.*}} and {{bm.*.*}} holes; boards with other holes show them raw.
"""
import json, re, sys
from pathlib import Path

project = Path(__file__).resolve().parent.parent / 'project'
script = (project / 'sf-symbols.js').read_text()
symbols = json.loads(re.search(r'window\.SF_SYMBOLS = (\{.*?\});\n', script).group(1))
marks = json.loads(re.search(r'window\.BRAND_MARKS = (\{.*?\});\n', script).group(1))
out = Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
(out / 'an.css').write_text((project / 'an.css').read_text())
boards = sys.argv[2:] or [p.name for p in project.glob('*.dc.html')]

def fill(match):
	parts = match.group(1).strip().split('.')
	if parts[0] == 'ic':
		return symbols.get(parts[1], 'M0 0')
	if parts[0] == 'bm':
		return marks[parts[1]][parts[2]]
	return match.group(0)

for name in boards:
	html = (project / name).read_text()
	head = re.search(r'<title>.*?</title>', html, re.S).group(0)
	style = re.search(r'<helmet>(.*?)</helmet>', html, re.S).group(1)
	body = re.search(r'</helmet>(.*?)</x-dc>', html, re.S).group(1)
	page = f'<!doctype html><html><head><meta charset="utf-8">{head}<link rel="stylesheet" href="an.css">{style}</head><body style="background:#888;padding:20px">{body}</body></html>'
	(out / name.replace('.dc.html', '.html')).write_text(re.sub(r'\{\{([^}]+)\}\}', fill, page))
print('ok', len(boards))
