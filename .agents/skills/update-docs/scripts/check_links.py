"""Checks that relative links and image sources in Markdown files resolve, anchors included, and
that each repository path written as code in prose, such as `crates/sisuc/src/main.rs`, exists.

Usage: python3 check_links.py FILE.md... Prints each broken link or path and exits 1 if any.
"""
import re
import sys
from pathlib import Path

INLINE = re.compile(r'\]\(<?([^)\s>]+)>?(?:\s+"[^"]*")?\)')
REF_DEF = re.compile(r'^\s*\[[^\]]+\]:\s+<?(\S+?)>?(?:\s+"[^"]*")?\s*$', re.M)
ATTR = re.compile(r'\b(src|href|srcset)="([^"]+)"')
FENCE = re.compile(r'^(```|~~~).*?^\1[^\n]*$', re.M | re.S)
# A code span naming a path under one of the repository's top-level directories. Placeholders
# such as `<name>` and globs such as `*.md` do not match, so they are not checked.
REPO_PATH = re.compile(r'`((?:crates|docs|scripts|\.agents|\.cargo|\.config|\.github)/[\w./-]*)`')
ROOT = Path(__file__).resolve().parents[4]


def prose(path):
    """The file's text with fenced code blocks removed."""
    return FENCE.sub('', path.read_text())


def anchors(path):
    """GitHub's heading slugs: link text kept, punctuation dropped, duplicates numbered."""
    seen = {}
    for heading in re.findall(r'^#{1,6}\s+(.*?)\s*#*\s*$', prose(path), re.M):
        text = re.sub(r'!?\[([^\]]*)\]\([^)]*\)', r'\1', heading).replace('`', '')
        slug = re.sub(r'[^\w\- ]', '', text.lower()).replace(' ', '-')
        count = seen.get(slug, 0)
        seen[slug] = count + 1
        yield slug if count == 0 else f'{slug}-{count}'


def targets(text):
    for m in INLINE.finditer(text):
        yield m.group(1)
    for m in REF_DEF.finditer(text):
        yield m.group(1)
    for attr, value in ATTR.findall(text):
        if attr == 'srcset':
            yield from (part.split()[0] for part in value.split(',') if part.strip())
        else:
            yield value


bad = 0
for md in map(Path, sys.argv[1:]):
    for target in targets(prose(md)):
        if re.match(r'[a-z][a-z0-9+.-]*:', target):
            continue
        file, _, anchor = target.split('?')[0].partition('#')
        dest = (md.parent / file).resolve() if file else md.resolve()
        if not dest.exists():
            print(f'{md}: missing {target}')
            bad += 1
        elif anchor and dest.suffix == '.md' and anchor.lower() not in set(anchors(dest)):
            print(f'{md}: missing anchor {target}')
            bad += 1
    for path in REPO_PATH.findall(prose(md)):
        if not (ROOT / path).exists():
            print(f'{md}: missing path `{path}`')
            bad += 1
print('broken:', bad)
sys.exit(1 if bad else 0)
