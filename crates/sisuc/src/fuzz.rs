//! The entry point of the front-end fuzz target: `sisuc --check` on one input, with checks that
//! every span it reports is well formed. The bug signal is a panic.

use crate::diagnostic::{Diagnostic, Edit, Fix};
use crate::report::{Format, Reporter};

/// Runs `sisuc --check` on `data` as the file `input.sisu`, reports it as colored text and then
/// as JSON, and asserts that every span of every diagnostic is well formed. Returns the text
/// report followed by the JSON report.
///
/// The caller runs it on a thread with a stack of `STACK_SIZE`, as `main` runs `cli`: the
/// deepest program the parser accepts overflows a smaller stack.
pub fn front_end(data: &[u8]) -> String {
    let mut out = Vec::new();
    let mut text = Reporter::new(&mut out, Format::Text, true);
    let checked = crate::check_bytes(&mut text, "input.sisu", data.to_vec());
    text.finish();
    let mut json = Reporter::new(&mut out, Format::Json, false);
    json.diagnostics("input.sisu", &checked.source, &checked.diagnostics);
    json.finish();
    assert_spans(&checked.source, &checked.diagnostics);
    String::from_utf8(out).expect("the reports should be UTF-8, as the source they quote is")
}

/// Asserts that every span of `diagnostics` (primary, secondary and each fix's edits) is a range
/// of `source` whose ends sit on char boundaries, and that no two edits of one fix overlap.
fn assert_spans(source: &str, diagnostics: &[Diagnostic]) {
    let len = source.len();
    for d in diagnostics {
        let code = d.code().as_str();
        let secondary = d.secondary().iter().map(|&(span, _)| span);
        let edits = d.fixes().iter().flat_map(Fix::edits).map(Edit::span);
        for span in std::iter::once(d.span()).chain(secondary).chain(edits) {
            assert!(
                span.start <= span.end,
                "{code}: span {span:?} starts after it ends, in a source of {len} bytes"
            );
            // `is_char_boundary` is false past the end, so this also rejects such a span.
            assert!(
                source.is_char_boundary(span.start),
                "{code}: span {span:?} starts inside a char, in a source of {len} bytes"
            );
            assert!(
                source.is_char_boundary(span.end),
                "{code}: span {span:?} ends inside a char or past the source of {len} bytes"
            );
        }
        for fix in d.fixes() {
            let edits = fix.edits();
            for (i, a) in edits.iter().enumerate() {
                for b in &edits[i + 1..] {
                    let (a, b) = (a.span(), b.span());
                    assert!(
                        !(a.start < b.end && b.start < a.end),
                        "{code}: edits conflict: {a:?} and {b:?} of one fix overlap, in a source of {len} bytes"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::thread;

    use super::{assert_spans, front_end};
    use crate::diagnostic::{Code, Diagnostic, Span};

    /// `front_end` on a thread with a `STACK_SIZE` stack, as its contract asks.
    fn on_stack(data: &[u8]) -> String {
        thread::scope(|scope| {
            thread::Builder::new()
                .stack_size(crate::STACK_SIZE)
                .spawn_scoped(scope, || front_end(data))
                .expect("the OS should start a thread with a 64 MiB stack")
                .join()
                .expect("front_end should not panic")
        })
    }

    /// Every `.sisu` file under `dir`, recursively.
    fn seeds(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("the tests directory should be readable") {
            let path = entry.expect("a directory entry should be readable").path();
            if path.is_dir() {
                seeds(&path, found);
            } else if path.extension().is_some_and(|e| e == "sisu") {
                found.push(path);
            }
        }
    }

    #[test]
    fn every_seed_passes() {
        let mut found = Vec::new();
        seeds(
            Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests")),
            &mut found,
        );
        assert!(found.len() > 300, "{} seeds", found.len());
        for path in found {
            on_stack(&fs::read(&path).expect("a seed should be readable"));
        }
    }

    #[test]
    fn reports_in_color_text_then_json() {
        let seed = fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/diagnostics/E0001-const.sisu"
        ))
        .expect("the seed should be readable");
        let out = on_stack(&seed);
        assert!(out.contains("\x1b["), "{out}");
        assert!(out.matches("E0001").count() >= 2, "{out}");
        let last = out.lines().last().expect("the report should not be empty");
        serde_json::from_str::<serde_json::Value>(last).expect("the last line should be JSON");
    }

    #[test]
    fn invalid_utf8_reports_over_the_replacement_text() {
        for data in [
            &b"\xff"[..],
            b"fn main() {}\n\xff",
            b"fn main() {}\n\xe2\x82",
        ] {
            let out = on_stack(data);
            assert!(out.contains(Code::InvalidUtf8.as_str()), "{out}");
        }
    }

    /// The deepest parentheses `tests/nesting.rs` compiles: 253 levels inside `print(`.
    #[test]
    fn deepest_nesting_fits_the_stack() {
        let n = 253;
        let source = format!(
            "fn main() {{\n    print({}1{})\n}}\n",
            "(".repeat(n),
            ")".repeat(n)
        );
        let out = on_stack(source.as_bytes());
        let summary: serde_json::Value =
            serde_json::from_str(out.lines().last().expect("the report should not be empty"))
                .expect("the summary should be JSON");
        assert_eq!(summary["errors"], 0, "{out}");
    }

    /// `aä b`: 5 bytes, and `ä` is bytes 1..3.
    const SOURCE: &str = "aä b";

    fn span(start: usize, end: usize) -> Span {
        Span::new(start, end)
    }

    /// A diagnostic at `0..1` with the one fix `edits`.
    fn fixed(edits: &[(Span, &str)]) -> Diagnostic {
        Diagnostic::for_test(span(0, 1), &[], edits)
    }

    #[test]
    fn accepts_spans_and_edits_at_the_edges() {
        assert_spans(
            SOURCE,
            &[
                Diagnostic::for_test(span(5, 5), &[], &[]),
                Diagnostic::for_test(span(0, 5), &[], &[]),
                Diagnostic::for_test(span(0, 1), &[span(3, 5)], &[]),
                // An insertion at the end of a replaced range, in either order.
                fixed(&[(span(0, 1), "x"), (span(1, 1), "y")]),
                fixed(&[(span(1, 1), "y"), (span(0, 1), "x")]),
                fixed(&[(span(5, 5), "a"), (span(5, 5), "b")]),
                // Adjacent replacements, in either order.
                fixed(&[(span(0, 1), ""), (span(1, 3), "")]),
                fixed(&[(span(1, 3), ""), (span(0, 1), "")]),
            ],
        );
    }

    #[test]
    #[should_panic(expected = "starts after it ends")]
    fn rejects_start_after_end() {
        assert_spans(SOURCE, &[Diagnostic::for_test(span(4, 3), &[], &[])]);
    }

    #[test]
    #[should_panic(expected = "ends inside a char")]
    fn rejects_end_past_the_source() {
        assert_spans(SOURCE, &[Diagnostic::for_test(span(4, 6), &[], &[])]);
    }

    #[test]
    #[should_panic(expected = "starts inside a char")]
    fn rejects_start_inside_a_char() {
        assert_spans(SOURCE, &[Diagnostic::for_test(span(2, 3), &[], &[])]);
    }

    #[test]
    #[should_panic(expected = "ends inside a char")]
    fn rejects_end_inside_a_char() {
        assert_spans(SOURCE, &[Diagnostic::for_test(span(0, 2), &[], &[])]);
    }

    #[test]
    #[should_panic(expected = "ends inside a char")]
    fn rejects_a_bad_secondary_span() {
        assert_spans(
            SOURCE,
            &[Diagnostic::for_test(span(0, 1), &[span(4, 6)], &[])],
        );
    }

    #[test]
    #[should_panic(expected = "starts inside a char")]
    fn rejects_a_bad_edit_span() {
        assert_spans(SOURCE, &[fixed(&[(span(2, 3), "")])]);
    }

    #[test]
    #[should_panic(expected = "edits conflict")]
    fn rejects_overlapping_edits() {
        assert_spans(SOURCE, &[fixed(&[(span(1, 4), ""), (span(0, 3), "")])]);
    }

    #[test]
    #[should_panic(expected = "edits conflict")]
    fn rejects_an_insertion_inside_a_replacement() {
        assert_spans(SOURCE, &[fixed(&[(span(0, 3), ""), (span(1, 1), "")])]);
    }
}
