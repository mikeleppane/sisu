//! Diagnostics: what the compiler reports about the source, and how it renders them.

#[cfg(test)]
pub(crate) mod explanations;
pub(crate) mod json;
mod registry;
mod render;
mod suggest;

pub(crate) use registry::{
    Code, Conversion, Definition, DiagnosticKind, Earlier, Immutable, Join, LetField, Literal,
    SemicolonPlace, Unwrap,
};
pub(crate) use suggest::{Suggestion, suggestion};

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

/// One error or warning about the source. Only `diagnostic` and its modules build one.
#[derive(Clone, Debug)]
pub(crate) struct Diagnostic {
    /// Its severity is the code's.
    code: Code,
    /// `Box<str>`, like `label` and `help`, keeps a `Diagnostic` under clippy's limit for a
    /// `Result`'s error.
    message: Box<str>,
    /// The offending code, underlined `^^^`.
    span: Span,
    /// Text under the primary span.
    label: Option<Box<str>>,
    /// Other spans that explain the error, underlined `---`.
    secondary: Vec<(Span, String)>,
    /// Advice that is not an edit.
    help: Option<Box<str>>,
    /// Alternative edits, each of which resolves the diagnostic.
    fixes: Vec<Fix>,
}

/// An edit to the source, with a message, that resolves a diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fix {
    message: String,
    /// Applying the edits always removes the diagnostic, and the source still parses.
    certain: bool,
    /// They apply together, to the original source, and never overlap.
    edits: Vec<Edit>,
}

/// Replaces `span` of the original source with `replacement`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Edit {
    span: Span,
    replacement: String,
}

impl Diagnostic {
    /// A diagnostic of `code`, with the code's severity.
    fn coded(code: Code, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            code,
            message: message.into().into_boxed_str(),
            span,
            label: None,
            secondary: Vec::new(),
            help: None,
            fixes: Vec::new(),
        }
    }

    fn with_label(mut self, text: impl Into<String>) -> Diagnostic {
        self.label = Some(text.into().into_boxed_str());
        self
    }

    fn with_secondary(mut self, span: Span, text: impl Into<String>) -> Diagnostic {
        self.secondary.push((span, text.into()));
        self
    }

    fn with_help(mut self, text: impl Into<String>) -> Diagnostic {
        self.help = Some(text.into().into_boxed_str());
        self
    }

    fn with_fix(mut self, fix: Fix) -> Diagnostic {
        self.fixes.push(fix);
        self
    }

    pub(crate) fn code(&self) -> Code {
        self.code
    }

    pub(crate) fn severity(&self) -> Severity {
        self.code.severity()
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn span(&self) -> Span {
        self.span
    }

    pub(crate) fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub(crate) fn secondary(&self) -> &[(Span, String)] {
        &self.secondary
    }

    pub(crate) fn help(&self) -> Option<&str> {
        self.help.as_deref()
    }

    pub(crate) fn fixes(&self) -> &[Fix] {
        &self.fixes
    }
}

impl Fix {
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn certain(&self) -> bool {
        self.certain
    }

    pub(crate) fn edits(&self) -> &[Edit] {
        &self.edits
    }
}

impl Edit {
    pub(crate) fn span(&self) -> Span {
        self.span
    }

    pub(crate) fn replacement(&self) -> &str {
        &self.replacement
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
}
