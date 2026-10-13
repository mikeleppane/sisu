"""Turns a cargo-llvm-cov export, its LCOV file and a git diff into the uncovered regions the
change touches.

Usage: python3 coverage_report.py COVERAGE.json LCOV.info CHANGES.diff
Exits 0 for any valid report, however low the coverage, and 1 when an input is unusable.
"""
import json
import re
import subprocess
import sys
from typing import Callable, NamedTuple

TEST_DIR = re.compile(r'^crates/[^/]+/tests/')
HUNK = re.compile(r'^@@ -\S+ \+(\d+)')


class Region(NamedTuple):
    start: int
    start_col: int
    end: int
    end_col: int
    never_run: bool


def changed_lines(diff: str) -> dict[str, set[int]]:
    """The new-side line numbers of added lines, per Rust file outside crates/*/tests/."""
    result: dict[str, set[int]] = {}
    path = None
    line = 0
    for text in diff.splitlines():
        if text.startswith('+++ '):
            name = text[4:].removeprefix('b/')
            ok = name.endswith('.rs') and not TEST_DIR.match(name)
            path = name if ok else None
        elif text.startswith('diff --git '):
            path = None
        elif m := HUNK.match(text):
            line = int(m[1])
        elif text.startswith('+'):
            if path:
                result.setdefault(path, set()).add(line)
            line += 1
        elif text.startswith(' '):
            line += 1
    return result


def relative(path: str, root: str) -> str | None:
    return path[len(root) + 1:] if path.startswith(root + '/') else None


def uncovered(export: dict, root: str) -> dict[str, list[Region]]:
    """Zero-count regions per file; a function that never ran is one never_run region."""
    # A source function is its file plus the span of its first region; its instantiations
    # have different mangled names, so they are grouped by that key and merged span by span.
    functions: dict[tuple, dict[tuple, int]] = {}
    for record in export['data'][0]['functions']:
        regions = [r for r in record['regions'] if r[7] == 0]
        if not regions:
            continue
        path = relative(record['filenames'][regions[0][5]], root)
        if path is None:
            continue
        spans = functions.setdefault((path, tuple(regions[0][:4])), {})
        for r in regions:
            span = tuple(r[:4])
            spans[span] = max(spans.get(span, 0), r[4])
    result: dict[str, list[Region]] = {}
    for (path, first), spans in functions.items():
        if spans[first] == 0:
            found = [Region(*first, True)]
        else:
            found = [Region(*s, False) for s, count in spans.items() if count == 0]
        result.setdefault(path, []).extend(found)
    return {p: sorted(rs) for p, rs in result.items()}


def line_hits(lcov: str, root: str) -> dict[str, dict[int, int]]:
    result: dict[str, dict[int, int]] = {}
    hits = None
    for text in lcov.splitlines():
        if text.startswith('SF:'):
            path = relative(text[3:], root)
            hits = result.setdefault(path, {}) if path else None
        elif text.startswith('DA:') and hits is not None:
            number, count = text[3:].split(',')[:2]
            hits[int(number)] = hits.get(int(number), 0) + int(count)
    return result


def report(export: dict, lcov: str, diff: str, root: str,
           read_line: Callable[[str, int], str]) -> str:
    totals = export['data'][0]['totals']
    changed = changed_lines(diff)
    hits = line_hits(lcov, root)
    instrumented = [h[n] for p, ns in changed.items() for n in ns if n in (h := hits.get(p, {}))]
    if instrumented:
        covered = sum(1 for h in instrumented if h > 0)
        pct = f'changed lines {100 * covered / len(instrumented):.1f}% ({covered} of {len(instrumented)})'
    else:
        pct = 'changed lines: none'
    head = f"Coverage: lines {totals['lines']['percent']:.1f}%, regions {totals['regions']['percent']:.1f}%, {pct}."

    touching, other = [], []
    for path, regions in sorted(uncovered(export, root).items()):
        if path not in changed:
            continue
        for r in regions:
            source = read_line(path, r.start).strip()[:80]
            if r.never_run:
                text = f'{path}:{r.start}: never run: {source}'
            else:
                text = f'{path}:{r.start}:{r.start_col}-{r.end}:{r.end_col}: {source}'
            hit = any(n in changed[path] for n in range(r.start, r.end + 1))
            (touching if hit else other).append(text)
    return (f'{head}\n\nUncovered regions touching changed code:\n' + '\n'.join(touching or ['(none)'])
            + '\n\nOther uncovered regions in changed files (advisory):\n' + '\n'.join(other or ['(none)']))


def main(argv: list[str]) -> int:
    try:
        coverage_path, lcov_path, diff_path = argv
        with open(coverage_path) as f:
            export = json.load(f)
        with open(lcov_path, errors='replace') as f:
            lcov = f.read()
        with open(diff_path, errors='replace') as f:
            diff = f.read()
        export['data'][0]['totals']['lines']['percent']
        export['data'][0]['totals']['regions']['percent']
        export['data'][0]['functions']
        root = subprocess.run(['git', 'rev-parse', '--show-toplevel'], capture_output=True,
                              text=True, check=True).stdout.strip()
    except (OSError, ValueError, LookupError, TypeError, subprocess.CalledProcessError) as e:
        print(f'coverage_report: {type(e).__name__}: {e}', file=sys.stderr)
        return 1
    cache: dict[str, list[str]] = {}

    def read_line(path: str, number: int) -> str:
        if path not in cache:
            try:
                with open(f'{root}/{path}', errors='replace') as f:
                    cache[path] = f.read().splitlines()
            except OSError:
                cache[path] = []
        lines = cache[path]
        return lines[number - 1] if 0 < number <= len(lines) else ''

    print(report(export, lcov, diff, root, read_line))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
