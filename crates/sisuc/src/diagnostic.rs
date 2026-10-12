//! Diagnostics: what the compiler reports about the source, and how it renders them.

use annotate_snippets::{AnnotationKind, Level, Renderer, Snippet};

/// A byte range of the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl Span {
    pub(crate) fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    /// The span from the start of `self` to the end of `other`.
    pub(crate) fn to(self, other: Span) -> Span {
        Span::new(self.start, other.end)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub(crate) struct Diagnostic {
    pub(crate) severity: Severity,
    pub(crate) message: String,
    /// The offending code, underlined `^^^`.
    pub(crate) span: Span,
    /// Text under the primary span.
    pub(crate) label: Option<String>,
    /// Other spans that explain the error, underlined `---`.
    pub(crate) secondary: Vec<(Span, String)>,
    pub(crate) help: Option<String>,
}

impl Diagnostic {
    pub(crate) fn error(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            message: message.into(),
            span,
            label: None,
            secondary: Vec::new(),
            help: None,
        }
    }

    pub(crate) fn warning(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(span, message)
        }
    }

    pub(crate) fn label(mut self, text: impl Into<String>) -> Diagnostic {
        self.label = Some(text.into());
        self
    }

    pub(crate) fn secondary(mut self, span: Span, text: impl Into<String>) -> Diagnostic {
        self.secondary.push((span, text.into()));
        self
    }

    pub(crate) fn help(mut self, text: impl Into<String>) -> Diagnostic {
        self.help = Some(text.into());
        self
    }

    /// Renders the diagnostic as `annotate-snippets` text; `path` is shown as given. `lines` is
    /// the index of `source`. The renderer rebuilds a map of every line it is handed, so it gets
    /// only the lines the spans cover, numbered from where they start.
    pub(crate) fn render(&self, path: &str, source: &str, lines: &LineIndex) -> String {
        let spans = || std::iter::once(&self.span).chain(self.secondary.iter().map(|(s, _)| s));
        for span in spans() {
            debug_assert!(
                source.is_char_boundary(span.start) && source.is_char_boundary(span.end),
                "span {span:?} does not sit on char boundaries"
            );
            // A span that swallows the line break underlines into the next line.
            debug_assert!(
                span.start == span.end || !source[..span.end].ends_with('\n'),
                "span {span:?} ends with a line break"
            );
        }
        let first = spans().map(|s| lines.render_line(source, s.start)).min();
        let last = spans().map(|s| lines.render_line(source, s.end)).max();
        let (first, last) = (
            first.expect("a diagnostic should have a primary span"),
            last.expect("a diagnostic should have a primary span"),
        );
        // Start one line early. The renderer sizes the line-number gutter from the newlines
        // before the last span's end, counted with `lines().count() - 1`, which reads an empty
        // prefix as one newline too many; the whole source never has an empty prefix there. A
        // window with a line before it matches, and folding hides that line. This is
        // `newline_count` in annotate-snippets 0.12.16 `render.rs`; recheck it on a version bump.
        let first = first.saturating_sub(1).max(1);
        let from = lines.0[first - 1];
        let to = lines.0.get(last).copied().unwrap_or(source.len());
        let level = match self.severity {
            Severity::Error => Level::ERROR,
            Severity::Warning => Level::WARNING,
        };
        let rebased = |span: &Span| span.start - from..span.end - from;
        let mut primary = AnnotationKind::Primary.span(rebased(&self.span));
        if let Some(label) = &self.label {
            primary = primary.label(label);
        }
        let mut snippet = Snippet::source(&source[from..to])
            .line_start(first)
            .path(path)
            .annotation(primary);
        for (span, text) in &self.secondary {
            snippet = snippet.annotation(AnnotationKind::Context.span(rebased(span)).label(text));
        }
        let mut group = level.primary_title(&self.message).element(snippet);
        if let Some(help) = &self.help {
            group = group.element(Level::HELP.message(help));
        }
        Renderer::plain().render(&[group])
    }
}

/// The byte offset where each line of a source starts, so that a position costs a binary
/// search, not a scan from the start.
pub(crate) struct LineIndex(Vec<usize>);

impl LineIndex {
    pub(crate) fn new(source: &str) -> LineIndex {
        let breaks = source.match_indices('\n').map(|(i, _)| i + 1);
        LineIndex(std::iter::once(0).chain(breaks).collect())
    }

    /// The 1-based line and column of byte `offset` in the `source` that built the index; the
    /// column counts chars, not bytes.
    pub(crate) fn line_col(&self, source: &str, offset: usize) -> (usize, usize) {
        // Line starts at or before `offset`; the first is 0, so there is at least one.
        let line = self.0.partition_point(|&start| start <= offset);
        let line_start = self.0[line - 1];
        (line, source[line_start..offset].chars().count() + 1)
    }

    /// The 1-based line that `annotate-snippets` puts byte `offset` on. A text that ends in a
    /// line break has no empty last line there, so an offset at its end sits on the line before.
    /// `source.len()` is the only offset that can lie past the last rendered line, so it is the
    /// only one clamped.
    fn render_line(&self, source: &str, offset: usize) -> usize {
        let ends_in_break = !source.is_empty() && self.0.last() == Some(&source.len());
        let lines = self.0.len() - usize::from(ends_in_break);
        self.0.partition_point(|&start| start <= offset).min(lines)
    }
}

/// The 1-based line and column of byte `offset`; the column counts chars, not bytes.
#[cfg(test)]
pub(crate) fn line_col(source: &str, offset: usize) -> (usize, usize) {
    LineIndex::new(source).line_col(source, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_counts_chars_not_bytes() {
        assert_eq!(line_col("ää x", 5), (1, 4));
        assert_eq!(line_col("ää\nx", 5), (2, 1));
        assert_eq!(line_col("", 0), (1, 1));
    }

    #[test]
    fn line_index_agrees_with_a_scan_at_every_offset() {
        // The reference counts from the start of the text, as `line_col` did before the index.
        fn scan(source: &str, offset: usize) -> (usize, usize) {
            let before = &source[..offset];
            let line = before.matches('\n').count() + 1;
            let line_start = before.rfind('\n').map_or(0, |i| i + 1);
            (line, before[line_start..].chars().count() + 1)
        }
        for source in [
            "",
            "ää x",
            "ää\nx",
            "\n",
            "a\n\nöö€b\n",
            "x\nyy\nä",
            "\n\nä\n",
        ] {
            let index = LineIndex::new(source);
            for offset in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
                assert_eq!(
                    index.line_col(source, offset),
                    scan(source, offset),
                    "{source:?} at {offset}"
                );
            }
        }
    }

    #[test]
    fn line_index_pins_first_last_and_post_break_positions() {
        let source = "ä\nbö€\nc";
        let index = LineIndex::new(source);
        assert_eq!(index.line_col(source, 0), (1, 1));
        // Right before and right after a line break.
        assert_eq!(index.line_col(source, 2), (1, 2));
        assert_eq!(index.line_col(source, 3), (2, 1));
        // After the multi-byte chars of line 2, then the break.
        assert_eq!(index.line_col(source, 9), (2, 4));
        assert_eq!(index.line_col(source, 10), (3, 1));
        // The end of the text, after the last character.
        assert_eq!(index.line_col(source, source.len()), (3, 2));
    }

    #[test]
    #[should_panic(expected = "ends with a line break")]
    fn span_over_a_line_break_is_rejected() {
        Diagnostic::error(Span::new(5, 6), "x").render(
            "a.sisu",
            "let y\n}",
            &LineIndex::new("let y\n}"),
        );
    }

    #[test]
    fn renders_warning() {
        let source = "fn main() {\n    let mut count = 0\n}\n";
        let rendered = Diagnostic::warning(Span::new(24, 29), "`count` is never reassigned")
            .help("declare it with `let`")
            .render("w.sisu", source, &LineIndex::new(source));
        assert!(rendered.starts_with("warning: `count` is never reassigned\n --> w.sisu:2:13"));
        assert!(rendered.ends_with("= help: declare it with `let`"));
    }

    #[test]
    fn renders_error_with_secondary_label_and_help() {
        let source = "fn main() {\n    let n = 0\n    n = 1\n}\n";
        let d = Diagnostic::error(Span::new(30, 35), "cannot assign to `n`")
            .label("cannot assign twice")
            .secondary(Span::new(20, 21), "declared with `let` here")
            .help("declare it with `let mut`");
        let expected = "\
error: cannot assign to `n`
 --> test.sisu:3:5
  |
2 |     let n = 0
  |         - declared with `let` here
3 |     n = 1
  |     ^^^^^ cannot assign twice
  |
  = help: declare it with `let mut`";
        assert_eq!(
            d.render("test.sisu", source, &LineIndex::new(source)),
            expected
        );
    }

    /// The rendering before the windowing: the renderer gets the whole source.
    fn render_whole(d: &Diagnostic, path: &str, source: &str) -> String {
        let level = match d.severity {
            Severity::Error => Level::ERROR,
            Severity::Warning => Level::WARNING,
        };
        let mut primary = AnnotationKind::Primary.span(d.span.start..d.span.end);
        if let Some(label) = &d.label {
            primary = primary.label(label);
        }
        let mut snippet = Snippet::source(source).path(path).annotation(primary);
        for (span, text) in &d.secondary {
            snippet = snippet.annotation(
                AnnotationKind::Context
                    .span(span.start..span.end)
                    .label(text),
            );
        }
        let mut group = level.primary_title(&d.message).element(snippet);
        if let Some(help) = &d.help {
            group = group.element(Level::HELP.message(help));
        }
        Renderer::plain().render(&[group])
    }

    /// The span of `from`'s first occurrence in `source` through the end of `to`'s.
    fn between(source: &str, from: &str, to: &str) -> Span {
        let start = source
            .find(from)
            .expect("`from` should occur in the source");
        let end = source.find(to).expect("`to` should occur in the source") + to.len();
        Span::new(start, end)
    }

    #[test]
    fn windowed_rendering_matches_rendering_the_whole_source() {
        let twelve = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\n";
        let mut cases: Vec<(&str, Diagnostic)> = vec![
            // The first line.
            (
                "ab\ncd\nef\n",
                Diagnostic::error(between("ab\ncd\nef\n", "ab", "ab"), "m").label("here"),
            ),
            // The last line, with and without a final newline.
            (
                "ab\ncd\nef\n",
                Diagnostic::error(between("ab\ncd\nef\n", "ef", "ef"), "m"),
            ),
            (
                "ab\ncd\nef",
                Diagnostic::error(between("ab\ncd\nef", "ef", "ef"), "m"),
            ),
            // Empty spans at the end of the text, after a final newline and after none.
            (
                "ab\ncd\n",
                Diagnostic::error(Span::new(6, 6), "m").label("end of file"),
            ),
            (
                "ab\ncd",
                Diagnostic::error(Span::new(5, 5), "m").label("end of file"),
            ),
            (
                "ab\ncd\n",
                Diagnostic::error(Span::new(2, 2), "m").label("end of line"),
            ),
            ("ab\n\ncd", Diagnostic::error(Span::new(3, 3), "m")),
            ("", Diagnostic::error(Span::new(0, 0), "m").label("empty")),
            ("\n", Diagnostic::error(Span::new(1, 1), "m")),
            // A span over several lines, with a secondary span before it and help after.
            (
                "fn f() {\n    a\n    b\n    c\n    d\n    e\n    f\n}\n",
                Diagnostic::error(
                    between(
                        "fn f() {\n    a\n    b\n    c\n    d\n    e\n    f\n}\n",
                        "a\n    b\n    c\n    d\n    e",
                        "e",
                    ),
                    "m",
                )
                .label("many lines")
                .secondary(Span::new(3, 4), "open")
                .help("close it"),
            ),
            // Non-ASCII text and CRLF line breaks.
            (
                "ää\r\nöö x\r\n€€",
                Diagnostic::error(between("ää\r\nöö x\r\n€€", "x", "€€"), "m"),
            ),
            ("ää\r\nöö\r\n", Diagnostic::warning(Span::new(11, 11), "m")),
            // An empty span at the start of line 10: the renderer sizes the gutter from the
            // newlines before the span, and an empty window prefix counts one too many.
            (
                "1\n2\n3\n4\n5\n6\n7\n8\n9\nx",
                Diagnostic::error(Span::new(18, 18), "m"),
            ),
            (
                "\n\n\n\n\n\n\r\n//\r\n€\r\n }",
                Diagnostic::error(Span::new(17, 17), "m"),
            ),
            // The line numbers grow from one digit to two.
            (
                twelve,
                Diagnostic::error(between(twelve, "l9", "l9"), "m")
                    .secondary(between(twelve, "l11", "l11"), "later"),
            ),
            (
                twelve,
                Diagnostic::error(between(twelve, "l1\n", "l1"), "m")
                    .secondary(between(twelve, "l12", "l12"), "far"),
            ),
            (
                twelve,
                Diagnostic::error(between(twelve, "l10", "l10"), "m")
                    .secondary(between(twelve, "l2", "l2"), "before"),
            ),
        ];
        // Every single-char and every empty span of one text, as primary.
        let text = "ä\nbö€\n\nc";
        for start in 0..=text.len() {
            for end in start..=text.len().min(start + 1) {
                let on_boundary = text.is_char_boundary(start) && text.is_char_boundary(end);
                if on_boundary && (start == end || !text[..end].ends_with('\n')) {
                    cases.push((text, Diagnostic::error(Span::new(start, end), "m")));
                }
            }
        }
        for (source, d) in &cases {
            assert_eq!(
                d.render("t.sisu", source, &LineIndex::new(source)),
                render_whole(d, "t.sisu", source),
                "{source:?} {d:?}"
            );
        }
    }

    /// `.config/nextest.toml` kills this test after 30 s, matching it by name: a rename must
    /// update the override there. Every render used to cost the whole file.
    #[test]
    fn rendering_many_diagnostics_in_a_large_file_is_linear() {
        const LINES: usize = 200_000;
        let source = "let x = 1\n".repeat(LINES);
        let lines = LineIndex::new(&source);
        let mut len = 0;
        for i in (0..LINES).step_by(10) {
            let start = i * 10;
            let d = Diagnostic::error(Span::new(start, start + 3), "m");
            len += d.render("big.sisu", &source, &lines).len();
        }
        assert!(len > 0);
    }
}
