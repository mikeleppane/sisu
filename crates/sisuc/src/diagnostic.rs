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

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "used by the checker, wired into the CLI in Task 11"
        )
    )]
    pub(crate) fn warning(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(span, message)
        }
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "used by the checker, wired into the CLI in Task 11"
        )
    )]
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

    /// Renders the diagnostic as `annotate-snippets` text; `path` is shown as given.
    pub(crate) fn render(&self, path: &str, source: &str) -> String {
        let level = match self.severity {
            Severity::Error => Level::ERROR,
            Severity::Warning => Level::WARNING,
        };
        let mut primary = AnnotationKind::Primary.span(self.span.start..self.span.end);
        if let Some(label) = &self.label {
            primary = primary.label(label);
        }
        let mut snippet = Snippet::source(source).path(path).annotation(primary);
        for (span, text) in &self.secondary {
            snippet = snippet.annotation(
                AnnotationKind::Context
                    .span(span.start..span.end)
                    .label(text),
            );
        }
        let mut group = level.primary_title(&self.message).element(snippet);
        if let Some(help) = &self.help {
            group = group.element(Level::HELP.message(help));
        }
        Renderer::plain().render(&[group])
    }
}

/// The 1-based line and column of byte `offset`; the column counts chars, not bytes.
pub(crate) fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[line_start..].chars().count() + 1)
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
    fn renders_warning() {
        let source = "fn main() {\n    var count = 0\n}\n";
        let rendered = Diagnostic::warning(Span::new(20, 25), "`count` is never reassigned")
            .help("declare it with `let`")
            .render("w.sisu", source);
        assert!(rendered.starts_with("warning: `count` is never reassigned\n --> w.sisu:2:9"));
        assert!(rendered.ends_with("= help: declare it with `let`"));
    }

    #[test]
    fn renders_error_with_secondary_label_and_help() {
        let source = "fn main() {\n    let n = 0\n    n = 1\n}\n";
        let d = Diagnostic::error(Span::new(30, 35), "cannot assign to `n`")
            .label("cannot assign twice")
            .secondary(Span::new(20, 21), "declared with `let` here")
            .help("declare it with `var`");
        let expected = "\
error: cannot assign to `n`
 --> test.sisu:3:5
  |
2 |     let n = 0
  |         - declared with `let` here
3 |     n = 1
  |     ^^^^^ cannot assign twice
  |
  = help: declare it with `var`";
        assert_eq!(d.render("test.sisu", source), expected);
    }
}
