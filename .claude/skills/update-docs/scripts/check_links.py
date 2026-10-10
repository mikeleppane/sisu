"""Checks that relative links and image sources in Markdown files resolve, anchors included."""
import re
import sys
from pathlib import Path

LINK = re.compile(r'\]\(([^)\s]+)\)|(?:src|srcset)="([^"]+)"')


def anchors(path):
    text = path.read_text()
    slugs = set()
    for heading in re.findall(r'^#+\s+(.*)$', text, re.M):
        slug = re.sub(r'[^\w\- ]', '', heading.lower()).replace(' ', '-')
        slugs.add(slug)
    return slugs


bad = 0
for md in map(Path, sys.argv[1:]):
    for m in LINK.finditer(md.read_text()):
        target = m.group(1) or m.group(2)
        if re.match(r'[a-z]+:', target):
            continue
        file, _, anchor = target.partition('#')
        dest = (md.parent / file).resolve() if file else md.resolve()
        if not dest.exists():
            print(f'{md}: missing {target}')
            bad += 1
        elif anchor and anchor not in anchors(dest):
            print(f'{md}: missing anchor {target}')
            bad += 1
print('broken:', bad)
sys.exit(1 if bad else 0)
