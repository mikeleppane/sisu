"""Unit tests for coverage_report.py: python3 -m unittest discover -s scripts -p 'test_*.py'."""
import contextlib
import io
import json
import os
import tempfile
import unittest

import coverage_report as cr

ROOT = "/r"
TOTALS = {"lines": {"percent": 87.34}, "regions": {"percent": 85.06}}


def func(name, regions, file="/r/a.rs"):
    """A function record; each region is (l1, c1, l2, c2, count)."""
    return {
        "name": name,
        "filenames": [file],
        "count": regions[0][4],
        "regions": [[*r, 0, 0, 0] for r in regions],
    }


def export(*functions):
    return {"data": [{"totals": TOTALS, "functions": list(functions)}]}


def diff(path="a.rs", start=1, count=1):
    body = "".join("+x\n" for _ in range(count))
    return f"diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -0,0 +{start},{count} @@\n{body}"


def lcov(**lines):
    """lcov({"a.rs": {1: 1, 2: 0}}) as text, rooted at /r."""
    return "".join(
        f"SF:/r/{p}\n" + "".join(f"DA:{n},{h}\n" for n, h in da.items()) + "end_of_record\n"
        for p, da in lines.items()
    )


def run(exp, lc="", df="", read=lambda p, n: f"line {n}"):
    return cr.report(exp, lc, df, ROOT, read)


def sections(text):
    head, touching, other = text.split("\n\n")
    return head, touching.splitlines()[1:], other.splitlines()[1:]


class ChangedLines(unittest.TestCase):
    def test_changed_lines_takes_added_lines_of_the_new_side(self):
        d = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -3,2 +3,3 @@\n+a\n+b\n+c\n"
        self.assertEqual(cr.changed_lines(d), {"a.rs": {3, 4, 5}})

    def test_changed_lines_skips_deleted_test_and_non_rust_files(self):
        d = (
            "diff --git a/b.rs b/b.rs\ndeleted file mode 100644\n--- a/b.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n"
            "diff --git a/crates/sisuc/tests/x.rs b/crates/sisuc/tests/x.rs\n--- a/crates/sisuc/tests/x.rs\n+++ b/crates/sisuc/tests/x.rs\n@@ -0,0 +1 @@\n+x\n"
            "diff --git a/docs/x.md b/docs/x.md\n--- a/docs/x.md\n+++ b/docs/x.md\n@@ -0,0 +1 @@\n+x\n"
            "diff --git a/c.rs b/d.rs\nsimilarity index 100%\nrename from c.rs\nrename to d.rs\n"
        )
        self.assertEqual(cr.changed_lines(d), {})


class Report(unittest.TestCase):
    def test_a_multiline_region_touching_one_changed_line_is_listed_first(self):
        exp = export(func("f", [(1, 1, 30, 1, 1), (10, 5, 14, 6, 0), (20, 1, 21, 2, 0)]))
        _, touching, other = sections(run(exp, df=diff(start=12)))
        self.assertEqual(len(touching), 1)
        self.assertTrue(touching[0].startswith("a.rs:10:5-14:6: "))
        self.assertEqual(len(other), 1)
        self.assertTrue(other[0].startswith("a.rs:20:1-21:2: "))

    def test_identical_regions_take_the_highest_count(self):
        exp = export(
            func("_RA", [(1, 1, 9, 1, 3), (5, 1, 6, 1, 0)]),
            func("_RB", [(1, 1, 9, 1, 3), (5, 1, 6, 1, 3)]),
        )
        _, touching, other = sections(run(exp, df=diff(start=5)))
        self.assertEqual((touching, other), (["(none)"], ["(none)"]))

    def test_a_never_run_function_is_one_line(self):
        exp = export(func("f", [(10, 1, 14, 2, 0), (11, 1, 11, 5, 0), (12, 1, 12, 5, 0)]))
        out = run(exp, df=diff(start=11))
        self.assertEqual(out.count("never run:"), 1)
        self.assertIn("a.rs:10: never run: line 10", out)

    def test_changed_lines_coverage_counts_only_instrumented_lines(self):
        out = run(export(), lcov(**{"a.rs": {1: 1, 2: 1, 3: 0}}), diff(start=1, count=4))
        self.assertIn("changed lines 66.7% (2 of 3).", out.splitlines()[0])

    def test_no_changed_rust_line_says_none(self):
        head, touching, other = sections(run(export(func("f", [(1, 1, 2, 1, 1)]))))
        self.assertTrue(head.endswith("changed lines: none."))
        self.assertEqual((touching, other), (["(none)"], ["(none)"]))
        self.assertEqual(head, "Coverage: lines 87.3%, regions 85.1%, changed lines: none.")

    def test_paths_outside_the_root_are_dropped(self):
        exp = export(func("f", [(1, 1, 5, 1, 1), (3, 1, 4, 1, 0)], file="/rustc/abc/lib.rs"))
        _, touching, other = sections(run(exp, df=diff(path="lib.rs", start=3)))
        self.assertEqual((touching, other), (["(none)"], ["(none)"]))

    def test_instantiations_with_different_names_are_one_function(self):
        spans = [(1, 1, 9, 1), (3, 1, 4, 1), (6, 1, 7, 1)]
        a = func("_RNvA", [(*s, 0) for s in spans])
        b = func("_RNvB", [(*spans[0], 4), (*spans[1], 4), (*spans[2], 0)])
        out = run(export(a, b), df=diff(start=6))
        self.assertNotIn("never run:", out)
        _, touching, other = sections(out)
        self.assertEqual(len(touching), 1)
        self.assertTrue(touching[0].startswith("a.rs:6:1-7:1: "))
        self.assertEqual(other, ["(none)"])

    def test_long_source_lines_are_cut_at_80_characters(self):
        exp = export(func("f", [(1, 1, 9, 1, 1), (3, 1, 4, 1, 0)]))
        out = run(exp, df=diff(start=3), read=lambda p, n: "  " + "x" * 100)
        self.assertIn(": " + "x" * 80 + "\n", out + "\n")


class Main(unittest.TestCase):
    def call(self, exp_text, lcov_text="", diff_text=""):
        with tempfile.TemporaryDirectory() as d:
            paths = []
            for name, text in (("c.json", exp_text), ("l.info", lcov_text), ("d.diff", diff_text)):
                paths.append(os.path.join(d, name))
                with open(paths[-1], "w") as f:
                    f.write(text)
            return self.run_main(paths)

    def run_main(self, paths):
        err = io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
            code = cr.main(paths)
        return code, err.getvalue()

    def test_a_missing_input_exits_1(self):
        code, err = self.run_main(["/nonexistent/c.json", "/nonexistent/l", "/nonexistent/d"])
        self.assertEqual(code, 1)
        self.assertTrue(err.startswith("coverage_report:"))

    def test_an_export_without_totals_exits_1(self):
        code, err = self.call(json.dumps({"data": [{}]}))
        self.assertEqual(code, 1)
        self.assertTrue(err.startswith("coverage_report:"))

    def test_a_valid_export_exits_0(self):
        code, _ = self.call(json.dumps(export()))
        self.assertEqual(code, 0)


if __name__ == "__main__":
    unittest.main()
