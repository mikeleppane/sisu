//! The lexer: source text to tokens.

use std::fmt::Write;
use std::iter::Peekable;
use std::str::CharIndices;

use crate::diagnostic::{Diagnostic, DiagnosticKind, Join, LineIndex, Span};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Ident(String),
    /// The magnitude of an integer literal: at most `MIN_MAGNITUDE`, which only a unary `-` accepts.
    Int(u64),
    Fn,
    Let,
    Mut,
    Var,
    If,
    Else,
    While,
    For,
    In,
    Return,
    Break,
    Continue,
    Class,
    NoneKw,
    Is,
    SelfKw,
    True,
    False,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Eq,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Arrow,
    Dot,
    DotDot,
    LBracket,
    RBracket,
    Semicolon,
    Question,
    QuestionDot,
    QuestionQuestion,
    Newline,
    Eof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) span: Span,
}

impl TokenKind {
    /// The variant name, as `--emit tokens` prints it.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            TokenKind::Ident(_) => "Ident",
            TokenKind::Int(_) => "Int",
            TokenKind::Fn => "Fn",
            TokenKind::Let => "Let",
            TokenKind::Mut => "Mut",
            TokenKind::Var => "Var",
            TokenKind::If => "If",
            TokenKind::Else => "Else",
            TokenKind::While => "While",
            TokenKind::For => "For",
            TokenKind::In => "In",
            TokenKind::Return => "Return",
            TokenKind::Break => "Break",
            TokenKind::Continue => "Continue",
            TokenKind::Class => "Class",
            TokenKind::NoneKw => "NoneKw",
            TokenKind::Is => "Is",
            TokenKind::SelfKw => "SelfKw",
            TokenKind::True => "True",
            TokenKind::False => "False",
            TokenKind::Plus => "Plus",
            TokenKind::Minus => "Minus",
            TokenKind::Star => "Star",
            TokenKind::Slash => "Slash",
            TokenKind::Percent => "Percent",
            TokenKind::PlusEq => "PlusEq",
            TokenKind::MinusEq => "MinusEq",
            TokenKind::StarEq => "StarEq",
            TokenKind::SlashEq => "SlashEq",
            TokenKind::PercentEq => "PercentEq",
            TokenKind::EqEq => "EqEq",
            TokenKind::NotEq => "NotEq",
            TokenKind::Lt => "Lt",
            TokenKind::Le => "Le",
            TokenKind::Gt => "Gt",
            TokenKind::Ge => "Ge",
            TokenKind::AndAnd => "AndAnd",
            TokenKind::OrOr => "OrOr",
            TokenKind::Bang => "Bang",
            TokenKind::Eq => "Eq",
            TokenKind::LParen => "LParen",
            TokenKind::RParen => "RParen",
            TokenKind::LBrace => "LBrace",
            TokenKind::RBrace => "RBrace",
            TokenKind::Comma => "Comma",
            TokenKind::Colon => "Colon",
            TokenKind::Arrow => "Arrow",
            TokenKind::Dot => "Dot",
            TokenKind::DotDot => "DotDot",
            TokenKind::LBracket => "LBracket",
            TokenKind::RBracket => "RBracket",
            TokenKind::Semicolon => "Semicolon",
            TokenKind::Question => "Question",
            TokenKind::QuestionDot => "QuestionDot",
            TokenKind::QuestionQuestion => "QuestionQuestion",
            TokenKind::Newline => "Newline",
            TokenKind::Eof => "Eof",
        }
    }
}

impl TokenKind {
    /// How a diagnostic names the token: "`)`", "`x`", "end of line".
    pub(crate) fn describe(&self) -> String {
        let text = match self {
            TokenKind::Ident(name) => name.as_str(),
            TokenKind::Int(n) => return format!("`{n}`"),
            TokenKind::Fn => "fn",
            TokenKind::Let => "let",
            TokenKind::Mut => "mut",
            TokenKind::Var => "var",
            TokenKind::If => "if",
            TokenKind::Else => "else",
            TokenKind::While => "while",
            TokenKind::For => "for",
            TokenKind::In => "in",
            TokenKind::Return => "return",
            TokenKind::Break => "break",
            TokenKind::Continue => "continue",
            TokenKind::Class => "class",
            TokenKind::NoneKw => "None",
            TokenKind::Is => "is",
            TokenKind::SelfKw => "self",
            TokenKind::True => "true",
            TokenKind::False => "false",
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Percent => "%",
            TokenKind::PlusEq => "+=",
            TokenKind::MinusEq => "-=",
            TokenKind::StarEq => "*=",
            TokenKind::SlashEq => "/=",
            TokenKind::PercentEq => "%=",
            TokenKind::EqEq => "==",
            TokenKind::NotEq => "!=",
            TokenKind::Lt => "<",
            TokenKind::Le => "<=",
            TokenKind::Gt => ">",
            TokenKind::Ge => ">=",
            TokenKind::AndAnd => "&&",
            TokenKind::OrOr => "||",
            TokenKind::Bang => "!",
            TokenKind::Eq => "=",
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBrace => "{",
            TokenKind::RBrace => "}",
            TokenKind::Comma => ",",
            TokenKind::Colon => ":",
            TokenKind::Arrow => "->",
            TokenKind::Dot => ".",
            TokenKind::DotDot => "..",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::Semicolon => ";",
            TokenKind::Question => "?",
            TokenKind::QuestionDot => "?.",
            TokenKind::QuestionQuestion => "??",
            TokenKind::Newline => return "end of line".to_string(),
            TokenKind::Eof => return "end of file".to_string(),
        };
        format!("`{text}`")
    }
}

type Chars<'a> = Peekable<CharIndices<'a>>;

/// An open bracket: its char (`(`, `[` or `{`) and where it stands.
type Open = (char, Span);

/// What the lexer makes of a source: its tokens, which always end with `Eof`, a diagnostic
/// for each error it recovered from, where each `//` comment starts, and where each skip that
/// may have swallowed an item starts.
#[derive(Debug)]
pub(crate) struct Lexed {
    pub(crate) tokens: Vec<Token>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) comments: Vec<usize>,
    pub(crate) swallowed: Vec<usize>,
}

/// The tokens of `source`, or its first diagnostic: for tests that need source that lexes clean.
#[cfg(test)]
pub(crate) fn lex_clean(source: &str) -> Result<Vec<Token>, Diagnostic> {
    let lexed = lex(source)?;
    match lexed.diagnostics.into_iter().next() {
        Some(d) => Err(d),
        None => Ok(lexed.tokens),
    }
}

/// Lexes `source`, reporting every error it can continue after. `Err` holds the one diagnostic a
/// bracket error leaves: the first recoverable diagnostic, or the bracket error when there is none.
pub(crate) fn lex(source: &str) -> Result<Lexed, Diagnostic> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut comments = Vec::new();
    let mut swallowed = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    let mut chars = source.char_indices().peekable();
    let mut line_start = true;
    // Where the last skipped text starts.
    let mut skipped = None;
    while let Some((start, c)) = chars.next() {
        let kind = match c {
            ' ' | '\t' | '\r' => continue,
            '\n' => {
                line_start = true;
                // Look ahead only where a `Newline` would be pushed: the next newlines of a
                // blank run then find `Newline` last and skip it, so no gap is scanned twice.
                if ends_statement(&tokens, &open) {
                    if let Some(gap) = continues_line(&source[start..]) {
                        // Jump to the `.`/`?.`, so a long gap is scanned once.
                        while chars.next_if(|&(i, _)| i < start + gap).is_some() {}
                    } else {
                        // Empty, at the line end: a span over `\n` renders as a two-line range,
                        // and one at `\n` after `\r` sits a column too far right.
                        let end = start - usize::from(source[..start].ends_with('\r'));
                        tokens.push(newline(Span::new(end, end)));
                    }
                }
                continue;
            }
            '/' if chars.next_if(|&(_, n)| n == '/').is_some() => {
                comments.push(start);
                while chars.next_if(|&(_, n)| n != '\n').is_some() {}
                continue;
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let end = take_while(&mut chars, source, |c| {
                    c.is_ascii_alphanumeric() || c == '_'
                });
                keyword_or_ident(&source[start..end])
            }
            c if c.is_ascii_digit() => {
                let end = take_while(&mut chars, source, |c| c.is_ascii_digit() || c == '_');
                TokenKind::Int(int_literal(source, start, end).unwrap_or_else(|d| {
                    diagnostics.push(d);
                    0
                }))
            }
            '-' if matches!(chars.peek(), Some(&(_, ' ' | '\t')))
                && tokens.last().is_some_and(|t| t.kind == TokenKind::Newline) =>
            {
                let at = Span::new(start, start + 1);
                if !after_skip(source, &tokens, skipped) {
                    diagnostics.push(DiagnosticKind::LeadingBinaryMinus { at }.into());
                }
                TokenKind::Minus
            }
            _ => {
                if let Some(kind) = symbol(c, &mut chars) {
                    kind
                } else {
                    diagnostics.push(unexpected(c, start));
                    if skip_string(c, &mut chars) {
                        swallowed.push(start);
                    }
                    skipped = Some(start);
                    continue;
                }
            }
        };
        let end = chars.peek().map_or(source.len(), |&(i, _)| i);
        let span = Span::new(start, end);
        if line_start {
            if let Some(d) = check_line_start(source, &tokens, &open, &kind, span) {
                // A `{` on its own line lexes as if written on the line above. No other line-start
                // error follows a `Newline`: each fires only inside `(` or `[`.
                if tokens.last().is_some_and(|t| t.kind == TokenKind::Newline) {
                    tokens.pop();
                }
                if !after_skip(source, &tokens, skipped) {
                    diagnostics.push(d);
                }
            }
            line_start = false;
        }
        if let Err(d) = track_bracket(&mut open, &kind, span) {
            return Err(first_or(diagnostics, d));
        }
        tokens.push(Token { kind, span });
    }
    if let Some(&(c, span)) = open.last() {
        let unclosed = DiagnosticKind::UnclosedBracket {
            bracket: c,
            at: span,
        };
        return Err(first_or(diagnostics, unclosed.into()));
    }
    if ends_statement(&tokens, &open) {
        tokens.push(newline(Span::new(source.len(), source.len())));
    }
    tokens.push(eof(&tokens));
    Ok(Lexed {
        tokens,
        diagnostics,
        comments,
        swallowed,
    })
}

/// The `Eof` after `tokens`: just after the last code, so "found end of file" points at its line.
fn eof(tokens: &[Token]) -> Token {
    let after_code = tokens
        .iter()
        .rfind(|t| t.kind != TokenKind::Newline)
        .map_or(0, |t| t.span.end);
    Token {
        kind: TokenKind::Eof,
        span: Span::new(after_code, after_code),
    }
}

/// The diagnostic a bracket error leaves: the first of `diagnostics`, since a skip may have
/// caused the bracket error (`print("abc) }` skips both closers), or else `bracket`.
fn first_or(diagnostics: Vec<Diagnostic>, bracket: Diagnostic) -> Diagnostic {
    diagnostics.into_iter().next().unwrap_or(bracket)
}

/// Whether text was `skipped` on the line of the last code token or after it. A line-start
/// diagnostic then reads text that recovery may have dropped, so it may be a consequence of the
/// skip's own diagnostic, and is not reported.
fn after_skip(source: &str, tokens: &[Token], skipped: Option<usize>) -> bool {
    let code = tokens.iter().rfind(|t| t.kind != TokenKind::Newline);
    skipped.zip(code).is_some_and(|(s, code)| {
        // `None` when the skip comes after the code token.
        source
            .get(s..code.span.start)
            .is_none_or(|before| !before.contains('\n'))
    })
}

/// The offset in `rest` of the `.` or `?.` that continues the line break at the start
/// of `rest`, past whitespace, blank lines and `//` comments; `None` if code of another
/// kind follows.
fn continues_line(rest: &str) -> Option<usize> {
    let mut code = rest;
    loop {
        // Only what the main loop skips: `trim_start` would also take Unicode whitespace.
        code = code.trim_start_matches([' ', '\t', '\r', '\n']);
        match code.strip_prefix("//") {
            Some(comment) => code = comment.split_once('\n').map_or("", |(_, after)| after),
            None => {
                // `..` starts a range, not a continuation.
                return ((code.starts_with('.') && !code.starts_with(".."))
                    || code.starts_with("?."))
                .then(|| rest.len() - code.len());
            }
        }
    }
}

fn newline(span: Span) -> Token {
    Token {
        kind: TokenKind::Newline,
        span,
    }
}

/// Whether a line break here ends a statement: the previous token can end one,
/// and the innermost open bracket, if any, is a block. A `;` is not such a token, but the break
/// after it must reach the parser, so that it can tell a `;` that ends a line from one with
/// code after it.
fn ends_statement(tokens: &[Token], open: &[Open]) -> bool {
    let ends = can_end_statement(tokens.last())
        || tokens
            .last()
            .is_some_and(|t| t.kind == TokenKind::Semicolon);
    ends && open.last().is_none_or(|&(c, _)| c == '{')
}

/// Whether `token` can end a statement (Go's rule).
fn can_end_statement(token: Option<&Token>) -> bool {
    use TokenKind::{
        Break, Continue, False, Ident, Int, NoneKw, Question, RBrace, RBracket, RParen, Return,
        SelfKw, True,
    };
    token.is_some_and(|t| {
        matches!(
            t.kind,
            Ident(_)
                | Int(_)
                | True
                | False
                | Return
                | Break
                | Continue
                | NoneKw
                | SelfKw
                | Question
                | RParen
                | RBracket
                | RBrace
        )
    })
}

/// Layout rules 2 and 3 where the parser cannot see them; `kind` starts a line.
/// A `{` only ever follows `fn …`, `if …`, `else` or `while …` on its own line, so a
/// line-initial `{` after a token that could end a statement, or after `else`, breaks
/// rule 2 at any depth. Inside `( )` and `[ ]`, where no `Newline` reaches the parser, so does
/// a line-initial `else` (rule 3), and a line-initial `[` after a token that could end a
/// statement, which would silently index the line above.
fn check_line_start(
    source: &str,
    tokens: &[Token],
    open: &[Open],
    kind: &TokenKind,
    span: Span,
) -> Option<Diagnostic> {
    match kind {
        TokenKind::LBrace => {
            let code = tokens.iter().rfind(|t| t.kind != TokenKind::Newline);
            match code {
                Some(code) if can_end_statement(Some(code)) || code.kind == TokenKind::Else => {
                    // Only a header takes the `{`: joining it to another statement almost never
                    // gives a valid one (`print(1) {`, `let x = 1 {`, `} {`). Inside `(` or `[`
                    // the moved `{` would still sit in the unclosed bracket.
                    let statement = last_statement(tokens);
                    let join = (open.last().is_none_or(|&(c, _)| c == '{')
                        && statement.first().is_some_and(is_header)
                        && header_awaits_brace(statement))
                    .then(|| Join {
                        gap: Span::new(code.span.end, span.start),
                        // The join deletes the gap, so a comment in it makes the join a guess.
                        certain: header_awaits_brace(line_of(source, tokens, code))
                            && source[code.span.end..span.start].trim().is_empty(),
                    });
                    Some(DiagnosticKind::BraceOnNewLine { at: span, join }.into())
                }
                _ => None,
            }
        }
        TokenKind::LBracket
            if open.last().is_some_and(|&(c, _)| c != '{') && can_end_statement(tokens.last()) =>
        {
            Some(DiagnosticKind::BracketOnNewLine { at: span }.into())
        }
        TokenKind::Else
            if open.last().is_some_and(|&(c, _)| c != '{') && can_end_statement(tokens.last()) =>
        {
            // Only a `}` can take the `else`; the lexer cannot tell whether its `if` has one.
            let join = tokens
                .last()
                .filter(|t| t.kind == TokenKind::RBrace)
                .map(|brace| Join {
                    gap: Span::new(brace.span.end, span.start),
                    certain: false,
                });
            Some(DiagnosticKind::ElseOnNewLine { at: span, join }.into())
        }
        _ => None,
    }
}

/// The tokens on the line of `code`, up to the end of `tokens`.
fn line_of<'t>(source: &str, tokens: &'t [Token], code: &Token) -> &'t [Token] {
    let before = tokens
        .iter()
        .rposition(|t| source[t.span.start.min(code.span.start)..code.span.start].contains('\n'));
    &tokens[before.map_or(0, |i| i + 1)..]
}

/// The statement that `tokens` ends with, before any line break after it: back to the last
/// line break that ended a statement, or to the `{` of the block it stands in. A break inside
/// brackets or before a continuation never reached `tokens`, so a statement can span lines.
fn last_statement(tokens: &[Token]) -> &[Token] {
    let end = tokens
        .iter()
        .rposition(|t| t.kind != TokenKind::Newline)
        .map_or(0, |i| i + 1);
    let mut depth = 0_usize;
    let start = tokens[..end].iter().rposition(|t| match t.kind {
        TokenKind::RBrace => {
            depth += 1;
            false
        }
        TokenKind::LBrace if depth > 0 => {
            depth -= 1;
            false
        }
        TokenKind::LBrace | TokenKind::Newline => depth == 0,
        _ => false,
    });
    &tokens[start.map_or(0, |i| i + 1)..end]
}

/// Whether `t` starts a header, which takes a `{` at the end of its line.
fn is_header(t: &Token) -> bool {
    matches!(
        t.kind,
        TokenKind::Fn
            | TokenKind::Class
            | TokenKind::If
            | TokenKind::Else
            | TokenKind::While
            | TokenKind::For
    )
}

/// Whether `line`, the code before a `{` that starts a line, holds a header keyword that has no
/// `{` or `}` after it yet: the `{` then opens that header. A keyword before a `}` either has its
/// block already or sits in a closed one.
fn header_awaits_brace(line: &[Token]) -> bool {
    line.iter()
        .rev()
        .find_map(|t| match t.kind {
            TokenKind::LBrace | TokenKind::RBrace => Some(false),
            _ => is_header(t).then_some(true),
        })
        .unwrap_or(false)
}

/// Pushes an opening bracket onto `open`, or checks that a closing one matches the top.
fn track_bracket(open: &mut Vec<Open>, kind: &TokenKind, span: Span) -> Result<(), Diagnostic> {
    let (closes, c) = match kind {
        TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
            let c = match kind {
                TokenKind::LParen => '(',
                TokenKind::LBracket => '[',
                _ => '{',
            };
            open.push((c, span));
            return Ok(());
        }
        TokenKind::RParen => ('(', ')'),
        TokenKind::RBracket => ('[', ']'),
        TokenKind::RBrace => ('{', '}'),
        _ => return Ok(()),
    };
    match open.pop() {
        Some((o, _)) if o == closes => Ok(()),
        opened => Err(DiagnosticKind::UnexpectedClosingBracket {
            bracket: c,
            at: span,
            opened,
        }
        .into()),
    }
}

/// One token per line, `line:col Kind text`; `Newline` and `Eof` print without text.
pub(crate) fn dump(source: &str, tokens: &[Token]) -> String {
    let mut out = String::new();
    let lines = LineIndex::new(source);
    for t in tokens {
        let (line, col) = lines.line_col(source, t.span.start);
        write!(out, "{line}:{col} {}", t.kind.name())
            .expect("writing to a `String` should never fail");
        if !matches!(t.kind, TokenKind::Newline | TokenKind::Eof) {
            write!(out, " {}", &source[t.span.start..t.span.end])
                .expect("writing to a `String` should never fail");
        }
        out.push('\n');
    }
    out
}

/// Consumes chars while `pred` holds and returns the byte offset after the last one.
fn take_while(chars: &mut Chars, source: &str, pred: fn(char) -> bool) -> usize {
    while chars.next_if(|&(_, c)| pred(c)).is_some() {}
    chars.peek().map_or(source.len(), |&(i, _)| i)
}

fn keyword_or_ident(text: &str) -> TokenKind {
    match text {
        "fn" => TokenKind::Fn,
        "let" => TokenKind::Let,
        "mut" => TokenKind::Mut,
        "var" => TokenKind::Var,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "while" => TokenKind::While,
        "for" => TokenKind::For,
        "in" => TokenKind::In,
        "return" => TokenKind::Return,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "class" => TokenKind::Class,
        "None" => TokenKind::NoneKw,
        "is" => TokenKind::Is,
        "self" => TokenKind::SelfKw,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        _ => TokenKind::Ident(text.to_string()),
    }
}

/// The magnitude of `i64::MIN`, the one literal that fits `i64` only after a unary `-`.
pub(crate) const MIN_MAGNITUDE: u64 = 1 << 63;

/// The one error for a literal that no `i64` holds, wherever it is found.
pub(crate) fn literal_too_large(at: Span) -> Diagnostic {
    DiagnosticKind::IntegerTooLarge { at }.into()
}

/// Parses `source[start..end]`, a run of digits and `_`, as a magnitude of at most `MIN_MAGNITUDE`.
fn int_literal(source: &str, start: usize, end: usize) -> Result<u64, Diagnostic> {
    let text = &source[start..end];
    for (i, _) in text.match_indices('_') {
        let is_digit = |j: usize| text.as_bytes().get(j).is_some_and(u8::is_ascii_digit);
        if !i.checked_sub(1).is_some_and(is_digit) || !is_digit(i + 1) {
            let at = start + i;
            return Err(DiagnosticKind::DigitSeparator {
                at: Span::new(at, at + 1),
            }
            .into());
        }
    }
    text.replace('_', "")
        .parse()
        .ok()
        .filter(|&n| n <= MIN_MAGNITUDE)
        .ok_or_else(|| literal_too_large(Span::new(start, end)))
}

/// The operator or punctuation starting with `c`, consuming a second char when it has one.
fn symbol(c: char, chars: &mut Chars) -> Option<TokenKind> {
    use TokenKind::{
        AndAnd, Arrow, Bang, Colon, Comma, Dot, DotDot, Eq, EqEq, Ge, Gt, LBrace, LBracket, LParen,
        Le, Lt, Minus, MinusEq, NotEq, OrOr, Percent, PercentEq, Plus, PlusEq, QuestionDot,
        QuestionQuestion, RBrace, RBracket, RParen, Semicolon, Slash, SlashEq, Star, StarEq,
    };
    let mut eat = |want| chars.next_if(|&(_, n)| n == want).is_some();
    Some(match c {
        '+' if eat('=') => PlusEq,
        '+' => Plus,
        '-' if eat('=') => MinusEq,
        '-' if eat('>') => Arrow,
        '-' => Minus,
        '*' if eat('=') => StarEq,
        '*' => Star,
        '/' if eat('=') => SlashEq,
        '/' => Slash,
        '%' if eat('=') => PercentEq,
        '%' => Percent,
        '=' if eat('=') => EqEq,
        '=' => Eq,
        '!' if eat('=') => NotEq,
        '!' => Bang,
        '<' if eat('=') => Le,
        '<' => Lt,
        '>' if eat('=') => Ge,
        '>' => Gt,
        '&' if eat('&') => AndAnd,
        '|' if eat('|') => OrOr,
        '(' => LParen,
        ')' => RParen,
        '{' => LBrace,
        '}' => RBrace,
        ',' => Comma,
        ':' => Colon,
        '.' if eat('.') => DotDot,
        '.' => Dot,
        '[' => LBracket,
        ']' => RBracket,
        ';' => Semicolon,
        '?' if eat('.') => QuestionDot,
        '?' if eat('?') => QuestionQuestion,
        '?' => TokenKind::Question,
        _ => return None,
    })
}

/// After a stray `c`, skips the rest of a string if `c` is a `"`: to the closing `"` on this line,
/// so a `)` inside the string survives, or else to the line end. Whether the skipped text holds
/// the word `fn` or `class`, so that it may have swallowed an item.
fn skip_string(c: char, chars: &mut Chars) -> bool {
    if c != '"' {
        return false;
    }
    let mut text = String::new();
    while let Some((_, n)) = chars.next_if(|&(_, n)| n != '"' && n != '\n') {
        text.push(n);
    }
    chars.next_if(|&(_, n)| n == '"');
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|word| word == "fn" || word == "class")
}

fn unexpected(c: char, at: usize) -> Diagnostic {
    // The span covers the whole char: annotate-snippets panics off a char boundary.
    DiagnosticKind::UnexpectedCharacter {
        c,
        at: Span::new(at, at + c.len_utf8()),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::explanations::{apply_fix, fix_edits};
    use crate::diagnostic::{Code, LineIndex, Span, line_col};

    /// The tokens of `src`, which lexes without a diagnostic.
    fn tokens(src: &str) -> Vec<Token> {
        lex_clean(src).expect("source lexes")
    }

    /// The diagnostics of `src`, which the lexer recovers from.
    fn recovered(src: &str) -> Vec<Diagnostic> {
        let lexed = lex(src).expect("the lexer recovers");
        assert!(!lexed.diagnostics.is_empty(), "{src} lexes clean");
        lexed.diagnostics
    }

    /// The one diagnostic of `src`, which stops the lexer.
    fn fatal(src: &str) -> Diagnostic {
        lex(src).expect_err("the lexer stops")
    }

    /// The one diagnostic of `src`, which the lexer recovers from.
    fn only(src: &str) -> Diagnostic {
        let ds = recovered(src);
        let [d] = <[Diagnostic; 1]>::try_from(ds).expect("one diagnostic");
        d
    }

    fn at(src: &str, d: &Diagnostic) -> (Code, usize, usize) {
        let (line, col) = line_col(src, d.span().start);
        (d.code(), line, col)
    }

    fn err(src: &str) -> (Code, usize, usize) {
        at(src, &only(src))
    }

    fn kinds(src: &str) -> Vec<TokenKind> {
        tokens(src)
            .into_iter()
            .map(|t| t.kind)
            .filter(|k| *k != TokenKind::Newline)
            .collect()
    }

    fn ident(s: &str) -> TokenKind {
        TokenKind::Ident(s.to_string())
    }

    #[test]
    fn keywords_and_symbols() {
        use TokenKind::{Arrow, Colon, Eof, Fn, LBrace, LParen, RBrace, RParen};
        assert_eq!(
            kinds("fn f(a: i64) -> bool { a }"),
            [
                Fn,
                ident("f"),
                LParen,
                ident("a"),
                Colon,
                ident("i64"),
                RParen,
                Arrow,
                ident("bool"),
                LBrace,
                ident("a"),
                RBrace,
                Eof
            ]
        );
    }

    #[test]
    fn all_keywords() {
        use TokenKind::{Else, Eof, False, Fn, If, Let, Mut, Return, True, Var, While};
        assert_eq!(
            kinds("fn let mut var if else while return true false"),
            [Fn, Let, Mut, Var, If, Else, While, Return, True, False, Eof]
        );
    }

    #[test]
    fn break_and_continue_end_a_statement() {
        use TokenKind::{Break, Continue, Eof, Newline};
        assert_eq!(
            all_kinds("break\ncontinue"),
            [Break, Newline, Continue, Newline, Eof]
        );
    }

    #[test]
    fn class_and_optional_tokens() {
        use TokenKind::{
            Class, Colon, Dot, Eof, Is, Let, Mut, NoneKw, Question, QuestionDot, QuestionQuestion,
            SelfKw,
        };
        assert_eq!(
            kinds("class None is self"),
            [Class, NoneKw, Is, SelfKw, Eof]
        );
        assert_eq!(kinds("x?.f"), [ident("x"), QuestionDot, ident("f"), Eof]);
        assert_eq!(
            kinds("x ?? y"),
            [ident("x"), QuestionQuestion, ident("y"), Eof]
        );
        assert_eq!(kinds("Tree??"), [ident("Tree"), QuestionQuestion, Eof]);
        assert_eq!(kinds("Tree? ?"), [ident("Tree"), Question, Question, Eof]);
        assert_eq!(kinds("a.b"), [ident("a"), Dot, ident("b"), Eof]);
        assert_eq!(
            all_kinds("mut next: Tree?\nlet"),
            [
                Mut,
                ident("next"),
                Colon,
                ident("Tree"),
                Question,
                TokenKind::Newline,
                Let,
                Eof
            ]
        );
    }

    #[test]
    fn none_and_self_end_a_statement() {
        use TokenKind::{Eof, Eq, Newline, NoneKw, SelfKw};
        assert_eq!(
            all_kinds("x = None\ny"),
            [ident("x"), Eq, NoneKw, Newline, ident("y"), Newline, Eof]
        );
        assert_eq!(
            all_kinds("self\ny"),
            [SelfKw, Newline, ident("y"), Newline, Eof]
        );
    }

    #[test]
    fn dot_and_question_dot_continue_a_line() {
        use TokenKind::{
            Dot, Else, Eof, Eq, If, Int, LBrace, LParen, Let, Newline, QuestionDot,
            QuestionQuestion, RBrace, RParen,
        };
        assert_eq!(
            all_kinds("let t = a\n    .sum()"),
            [
                Let,
                ident("t"),
                Eq,
                ident("a"),
                Dot,
                ident("sum"),
                LParen,
                RParen,
                Newline,
                Eof
            ]
        );
        assert_eq!(
            all_kinds("a\r\n.b"),
            [ident("a"), Dot, ident("b"), Newline, Eof]
        );
        assert_eq!(
            all_kinds("a\r\n\r\n.b"),
            [ident("a"), Dot, ident("b"), Newline, Eof]
        );
        assert_eq!(all_kinds("a\n// c"), [ident("a"), Newline, Eof]);
        assert_eq!(
            all_kinds("a\n\n    // c\n    ?.b"),
            [ident("a"), QuestionDot, ident("b"), Newline, Eof]
        );
        assert_eq!(
            all_kinds("a\n?? b"),
            [
                ident("a"),
                Newline,
                QuestionQuestion,
                ident("b"),
                Newline,
                Eof
            ]
        );
        assert_eq!(all_kinds("a\n."), [ident("a"), Dot, Eof]);
        // Inside brackets: a block in parentheses, and parentheses alone.
        assert_eq!(
            all_kinds("f(if c {\n a\n .b\n } else { 0 })"),
            [
                ident("f"),
                LParen,
                If,
                ident("c"),
                LBrace,
                ident("a"),
                Dot,
                ident("b"),
                Newline,
                RBrace,
                Else,
                LBrace,
                Int(0),
                RBrace,
                RParen,
                Newline,
                Eof
            ]
        );
        assert_eq!(
            all_kinds("(a\n.b)"),
            [LParen, ident("a"), Dot, ident("b"), RParen, Newline, Eof]
        );
    }

    #[test]
    fn all_operators() {
        use TokenKind::{
            AndAnd, Bang, Comma, Eof, Eq, EqEq, Ge, Gt, Le, Lt, Minus, MinusEq, NotEq, OrOr,
            Percent, PercentEq, Plus, PlusEq, Slash, SlashEq, Star, StarEq,
        };
        assert_eq!(
            kinds("+ - * / % += -= *= /= %= == != < <= > >= && || ! = ,"),
            [
                Plus, Minus, Star, Slash, Percent, PlusEq, MinusEq, StarEq, SlashEq, PercentEq,
                EqEq, NotEq, Lt, Le, Gt, Ge, AndAnd, OrOr, Bang, Eq, Comma, Eof
            ]
        );
    }

    #[test]
    fn digit_separators() {
        assert_eq!(kinds("100_000"), [TokenKind::Int(100_000), TokenKind::Eof]);
    }

    #[test]
    fn double_underscore() {
        assert_eq!(err("1__0"), (Code::DigitSeparator, 1, 2));
    }

    #[test]
    fn trailing_underscore() {
        assert_eq!(err("100_"), (Code::DigitSeparator, 1, 4));
    }

    #[test]
    fn max_literal() {
        assert_eq!(
            kinds("9223372036854775807"),
            [TokenKind::Int(9_223_372_036_854_775_807), TokenKind::Eof]
        );
    }

    /// The magnitude of `i64::MIN` lexes: the parser decides whether a `-` precedes it.
    #[test]
    fn min_magnitude_literal() {
        assert_eq!(
            kinds("9223372036854775808"),
            [TokenKind::Int(MIN_MAGNITUDE), TokenKind::Eof]
        );
    }

    #[test]
    fn literal_too_large() {
        assert_eq!(err("9223372036854775809"), (Code::IntegerTooLarge, 1, 1));
        assert_eq!(
            err("99999999999999999999999"),
            (Code::IntegerTooLarge, 1, 1)
        );
    }

    #[test]
    fn unexpected_character_fix() {
        // A lone `&` or `|` is a guess at the doubled operator.
        for (src, doubled) in [("a & b", "&&"), ("a | b", "||")] {
            let d = only(src);
            assert_eq!(d.code(), Code::UnexpectedCharacter, "{src}");
            assert_eq!(line_col(src, d.span().start), (1, 3), "{src}");
            assert_eq!(
                fix_edits(&d),
                Some((false, vec![(2, 3, doubled.to_string())])),
                "{src}"
            );
        }
        // Neither is guessed for any other character.
        let d = only("a $ b");
        assert_eq!(d.code(), Code::UnexpectedCharacter);
        assert_eq!(fix_edits(&d), None);
        assert_eq!(d.help(), None);
    }

    #[test]
    fn unknown_char_counts_chars() {
        let src = "// ä\nlet é = 1";
        let d = only(src);
        assert_eq!(d.code(), Code::UnexpectedCharacter);
        assert_eq!(line_col(src, d.span().start), (2, 5));
        // The span covers the whole two-byte char, so rendering does not panic.
        assert!(
            d.render("t.sisu", src, &LineIndex::new(src), false)
                .contains("2:5")
        );
    }

    #[test]
    fn comment_skipped() {
        assert_eq!(kinds("x // päivää"), [ident("x"), TokenKind::Eof]);
    }

    #[test]
    fn token_spans_are_byte_offsets() {
        // The two-byte `ä` shifts byte offsets away from char offsets.
        let spans: Vec<Span> = tokens("// ä\nab 12 +=")
            .into_iter()
            .map(|t| t.span)
            .collect();
        assert_eq!(
            spans,
            [
                Span::new(6, 8),
                Span::new(9, 11),
                Span::new(12, 14),
                Span::new(14, 14)
            ]
        );
    }

    #[test]
    fn continuation_spans_skip_the_gap() {
        let spans = |src| -> Vec<Span> { tokens(src).into_iter().map(|t| t.span).collect() };
        assert_eq!(
            spans("a\n  .b"),
            [
                Span::new(0, 1),
                Span::new(4, 5),
                Span::new(5, 6),
                Span::new(6, 6),
                Span::new(6, 6)
            ]
        );
        assert_eq!(
            spans("a\n// c\n  .b"),
            [
                Span::new(0, 1),
                Span::new(9, 10),
                Span::new(10, 11),
                Span::new(11, 11),
                Span::new(11, 11)
            ]
        );
    }

    /// `.config/nextest.toml` kills this test after 30 s, matching it by name: a rename must
    /// update the override there.
    #[test]
    fn long_gaps_lex_correctly() {
        use TokenKind::{Dot, Eof, Newline};
        const LINES: usize = 200_000;
        for gap in ["\n", "\n// c"] {
            for (tail, expected) in [
                ("b", vec![ident("a"), Newline, ident("b"), Newline, Eof]),
                (".b", vec![ident("a"), Dot, ident("b"), Newline, Eof]),
            ] {
                let src = format!("a{}\n{tail}", gap.repeat(LINES));
                assert_eq!(all_kinds(&src), expected, "gap {gap:?} then {tail}");
            }
            // A continuation, then another long gap before a new statement.
            let src = format!("a{0}\n.b{0}\nc", gap.repeat(LINES));
            let expected = [
                ident("a"),
                Dot,
                ident("b"),
                Newline,
                ident("c"),
                Newline,
                Eof,
            ];
            assert_eq!(all_kinds(&src), expected, "gap {gap:?}, continuation, gap");
        }
    }

    #[test]
    fn unicode_space_in_the_gap_is_unexpected() {
        let src = "a\n\u{00a0}.b";
        assert_eq!(err(src), (Code::UnexpectedCharacter, 2, 1));
    }

    /// The `lex_*` fixtures pin how each is named.
    #[test]
    fn invisible_and_visible_characters_are_unexpected() {
        for c in [
            '\u{00a0}',
            '\u{2028}',
            '\u{0085}',
            '\u{0001}',
            '\u{007f}',
            '\u{00ad}',
            '\u{200b}',
            '\u{202e}',
            '\u{feff}',
            '\u{e0020}',
            '$',
            '\u{e9}',
            '\u{3042}',
            '\u{1f600}',
        ] {
            let src = format!("a {c}");
            assert_eq!(err(&src), (Code::UnexpectedCharacter, 1, 3), "{c:?}");
        }
    }

    #[test]
    fn byte_order_mark_is_removed_only_at_the_start_of_the_file() {
        let src = "\u{feff}fn main() {}";
        let d = only(src);
        assert_eq!(at(src, &d), (Code::UnexpectedCharacter, 1, 1));
        assert_eq!(fix_edits(&d), Some((true, vec![(0, 3, String::new())])));
        joined_parses(src, &d);
        for src in ["fn\u{feff}", "\n\u{feff}"] {
            assert_eq!(fix_edits(&only(src)), None, "{src:?}");
        }
    }

    fn all_kinds(src: &str) -> Vec<TokenKind> {
        tokens(src).into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn newline_after_rparen() {
        use TokenKind::{Eof, LParen, Newline, RParen};
        assert_eq!(
            all_kinds("f()\nx"),
            [
                ident("f"),
                LParen,
                RParen,
                Newline,
                ident("x"),
                Newline,
                Eof
            ]
        );
    }

    #[test]
    fn no_newline_inside_parens() {
        use TokenKind::{Eof, Int, LParen, Newline, Plus, RParen};
        assert_eq!(
            all_kinds("f(1\n+2)"),
            [
                ident("f"),
                LParen,
                Int(1),
                Plus,
                Int(2),
                RParen,
                Newline,
                Eof
            ]
        );
    }

    #[test]
    fn block_inside_parens() {
        use TokenKind::{Eof, LBrace, LParen, Newline, RBrace, RParen};
        assert_eq!(
            all_kinds("(\n{\nx\ny\n}\n)"),
            [
                LParen,
                LBrace,
                ident("x"),
                Newline,
                ident("y"),
                Newline,
                RBrace,
                RParen,
                Newline,
                Eof
            ]
        );
    }

    #[test]
    fn newline_after_comment() {
        use TokenKind::{Eof, Newline};
        assert_eq!(
            all_kinds("x // hi\ny"),
            [ident("x"), Newline, ident("y"), Newline, Eof]
        );
    }

    #[test]
    fn no_newline_after_operator() {
        use TokenKind::{Eof, Int, Newline, Plus};
        assert_eq!(all_kinds("1 +\n2"), [Int(1), Plus, Int(2), Newline, Eof]);
    }

    #[test]
    fn blank_lines_give_one_newline() {
        use TokenKind::{Eof, Newline};
        assert_eq!(
            all_kinds("x\n\n\ny"),
            [ident("x"), Newline, ident("y"), Newline, Eof]
        );
    }

    #[test]
    fn newline_at_eof() {
        use TokenKind::{Eof, Newline};
        assert_eq!(all_kinds("x"), [ident("x"), Newline, Eof]);
    }

    #[test]
    fn crlf_line_endings() {
        use TokenKind::{Eof, Newline};
        assert_eq!(
            all_kinds("x\r\ny\r\n"),
            [ident("x"), Newline, ident("y"), Newline, Eof]
        );
    }

    #[test]
    fn newline_and_eof_spans() {
        // A Newline sits empty at the line end, before `\r\n` or `\n`, or at end of file;
        // Eof sits just after the last code.
        let spans = |src| -> Vec<Span> { tokens(src).into_iter().map(|t| t.span).collect() };
        assert_eq!(
            spans("x // c\n"),
            [Span::new(0, 1), Span::new(6, 6), Span::new(1, 1)]
        );
        assert_eq!(
            spans("x\r\n"),
            [Span::new(0, 1), Span::new(1, 1), Span::new(1, 1)]
        );
        assert_eq!(
            spans("x"),
            [Span::new(0, 1), Span::new(1, 1), Span::new(1, 1)]
        );
    }

    #[test]
    fn unary_minus_line_is_fine() {
        use TokenKind::{Eof, Minus, Newline};
        assert_eq!(
            all_kinds("x\n-y"),
            [ident("x"), Newline, Minus, ident("y"), Newline, Eof]
        );
    }

    #[test]
    fn binary_minus_line_is_error() {
        use TokenKind::{Eof, Eq, Let, Minus, Newline};
        let src = "let t = a\n- b";
        assert_eq!(err(src), (Code::LeadingBinaryMinus, 2, 1));
        // The `-` is kept as a `Minus`.
        let kinds: Vec<_> = lex(src)
            .expect("the lexer recovers")
            .tokens
            .into_iter()
            .map(|t| t.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                Let,
                ident("t"),
                Eq,
                ident("a"),
                Newline,
                Minus,
                ident("b"),
                Newline,
                Eof
            ]
        );
    }

    #[test]
    fn unexpected_close() {
        assert_eq!(
            at("x)", &fatal("x)")),
            (Code::UnexpectedClosingBracket, 1, 2)
        );
    }

    #[test]
    fn mismatched_close() {
        let d = fatal("(}");
        let [(open, _)] = d.secondary() else {
            panic!("one secondary span, got {:?}", d.secondary());
        };
        assert_eq!(line_col("(}", open.start), (1, 1));
        // With nothing open, there is nothing to point at.
        let d = fatal("}");
        assert_eq!(d.secondary(), []);
    }

    #[test]
    fn unclosed_paren() {
        assert_eq!(at("(x", &fatal("(x")), (Code::UnclosedBracket, 1, 1));
    }

    #[test]
    fn binary_minus_inside_parens_is_fine() {
        use TokenKind::{Eof, LParen, Minus, Newline, RParen};
        assert_eq!(
            all_kinds("(a\n- b)"),
            [LParen, ident("a"), Minus, ident("b"), RParen, Newline, Eof]
        );
    }

    #[test]
    fn binary_minus_after_operator_is_fine() {
        use TokenKind::{Eof, Minus, Newline};
        assert_eq!(
            all_kinds("a -\n- b"),
            [ident("a"), Minus, Minus, ident("b"), Newline, Eof]
        );
    }

    #[test]
    fn bracket_tokens_and_keywords() {
        use TokenKind::{DotDot, Eof, For, In, Int, LBracket, RBracket, Semicolon};
        assert_eq!(
            kinds("[1, 2][0]"),
            [
                LBracket,
                Int(1),
                TokenKind::Comma,
                Int(2),
                RBracket,
                LBracket,
                Int(0),
                RBracket,
                Eof
            ]
        );
        assert_eq!(
            kinds("[0; 3]"),
            [LBracket, Int(0), Semicolon, Int(3), RBracket, Eof]
        );
        assert_eq!(kinds("for x in"), [For, ident("x"), In, Eof]);
        assert_eq!(kinds("0..n"), [Int(0), DotDot, ident("n"), Eof]);
        assert_eq!(kinds("a.b"), [ident("a"), TokenKind::Dot, ident("b"), Eof]);
    }

    #[test]
    fn brackets_suppress_newlines() {
        use TokenKind::{Comma, Eof, Int, LBracket, Newline, RBracket};
        assert_eq!(
            all_kinds("[\n1,\n2\n]"),
            [LBracket, Int(1), Comma, Int(2), RBracket, Newline, Eof]
        );
        // The only `Newline` is the one after the closing `]`.
        let nested = all_kinds("[\n    [1],\n    [2],\n]");
        assert_eq!(nested.iter().filter(|k| **k == Newline).count(), 1);
        assert_eq!(nested[nested.len() - 2], Newline);
    }

    #[test]
    fn block_inside_brackets_separates_statements() {
        let kinds = all_kinds("[\n    if c {\n        let n = 1\n        n\n    } else { 0 },\n]");
        let one = kinds.iter().position(|k| *k == TokenKind::Int(1));
        assert_eq!(
            one.map(|i| &kinds[i + 1]),
            Some(&TokenKind::Newline),
            "a `Newline` should follow `1` inside the block"
        );
    }

    #[test]
    fn bracket_line_start_outside_brackets_is_a_new_statement() {
        use TokenKind::{Eof, Int, LBracket, Newline, RBracket};
        assert_eq!(
            all_kinds("a\n[0]"),
            [
                ident("a"),
                Newline,
                LBracket,
                Int(0),
                RBracket,
                Newline,
                Eof
            ]
        );
    }

    #[test]
    fn range_is_not_a_continuation() {
        use TokenKind::{DotDot, Eof, Newline};
        assert_eq!(
            all_kinds("a\n..b"),
            [ident("a"), Newline, DotDot, ident("b"), Newline, Eof]
        );
    }

    #[test]
    fn bracket_on_new_line_inside_brackets() {
        for src in ["fn main() { (a\n[0]) }", "fn main() { [a\n[0]] }"] {
            let d = only(src);
            assert_eq!(d.code(), Code::BracketOnNewLine, "{src}");
            assert_eq!(d.span(), Span::new(15, 16), "{src}");
            assert!(keeps_token(src, &TokenKind::LBracket, d.span()), "{src}");
        }
    }

    /// Whether the tokens of `src` hold a `kind` at `span`.
    fn keeps_token(src: &str, kind: &TokenKind, span: Span) -> bool {
        let lexed = lex(src).expect("the lexer recovers");
        lexed
            .tokens
            .iter()
            .any(|t| t.kind == *kind && t.span == span)
    }

    #[test]
    fn else_on_new_line_inside_brackets() {
        assert_eq!(
            err("fn main() { [if c { 1 }\nelse { 2 }] }"),
            (Code::ElseOnNewLine, 2, 1)
        );
    }

    #[test]
    fn else_on_new_line_inside_parens() {
        assert_eq!(
            err("fn main() { print(if true { 1 }\nelse { 2 }) }"),
            (Code::ElseOnNewLine, 2, 1)
        );
    }

    #[test]
    fn else_on_new_line_fix() {
        // The gap after the `}` becomes one space; the lexer cannot tell the `if` has no `else`.
        let src = "fn main() { print(if true { 1 }\nelse { 2 }) }";
        let d = only(src);
        assert_eq!(
            fix_edits(&d),
            Some((false, vec![(31, 32, " ".to_string())]))
        );
        assert!(keeps_token(src, &TokenKind::Else, d.span()), "{src}");
        // Only a `}` before the `else` can take it.
        let d = only("fn main() { f(x\nelse y) }");
        assert_eq!(d.code(), Code::ElseOnNewLine);
        assert_eq!(fix_edits(&d), None);
    }

    #[test]
    fn brace_on_new_line_inside_parens() {
        assert_eq!(
            err("fn main() { print(if true\n{ 1 } else { 2 }) }"),
            (Code::BraceOnNewLine, 2, 1)
        );
    }

    #[test]
    fn brace_on_new_line_after_else() {
        assert_eq!(
            err("fn main() {\n    if true { print(1) } else\n    { print(2) }\n}"),
            (Code::BraceOnNewLine, 3, 5)
        );
    }

    #[test]
    fn brace_on_new_line_after_else_inside_parens() {
        assert_eq!(
            err("fn main() { print(if true { 1 } else\n{ 2 }) }"),
            (Code::BraceOnNewLine, 2, 1)
        );
    }

    /// Applies the one fix of `d` to `src` and asserts the result lexes and parses.
    fn joined_parses(src: &str, d: &Diagnostic) {
        let fixed = apply_fix(src, d);
        crate::parser::parse_ok(&fixed);
    }

    #[test]
    fn brace_on_new_line_fix() {
        // Every header keyword makes the fix certain, and the gap before the `{` becomes a space.
        for (src, gap, at) in [
            ("fn main()\n{\n}", (9, 10), (2, 1)),
            ("class A\n{\n}", (7, 8), (2, 1)),
            (
                "fn main() {\n    if true\n    {\n    }\n}",
                (23, 28),
                (3, 5),
            ),
            (
                "fn main() {\n    while true\n    {\n    }\n}",
                (26, 31),
                (3, 5),
            ),
            (
                "fn main() {\n    for i in xs\n    {\n    }\n}",
                (27, 32),
                (3, 5),
            ),
            (
                "fn main() {\n    if a {} else\n    {\n    }\n}",
                (28, 33),
                (3, 5),
            ),
            // A `let` inside a closed block is not on the header's line.
            (
                "fn main() {\n    if c { let y = 1 } else\n    {\n    }\n}",
                (39, 44),
                (3, 5),
            ),
        ] {
            let d = only(src);
            assert_eq!(d.code(), Code::BraceOnNewLine, "{src}");
            assert_eq!(line_col(src, d.span().start), at, "{src}");
            assert_eq!(
                fix_edits(&d),
                Some((true, vec![(gap.0, gap.1, " ".to_string())])),
                "{src}"
            );
            joined_parses(src, &d);
        }
    }

    #[test]
    fn brace_on_new_line_is_a_guess_over_a_comment() {
        // The join would delete the comment.
        let src = "fn main() {\n    if c // check it\n    {\n    }\n}";
        let d = only(src);
        assert_eq!(
            (d.code(), line_col(src, d.span().start)),
            (Code::BraceOnNewLine, (3, 5))
        );
        assert!(fix_edits(&d).is_some_and(|(certain, _)| !certain));
    }

    #[test]
    fn brace_on_new_line_is_a_guess_after_a_multi_line_header() {
        // The header keyword is on an earlier line of the same statement.
        for (src, at) in [
            (
                "fn main() {\n    if f(\n        1\n    )\n    {\n    }\n}",
                (5, 5),
            ),
            ("fn f(\n    x: i64\n)\n{\n}", (4, 1)),
            ("fn main() {\n    while\n        c\n    {\n    }\n}", (4, 5)),
            (
                "fn main() {\n    while a &&\n        b\n    {\n    }\n}",
                (4, 5),
            ),
            (
                "fn main() {\n    if xs\n        .is_empty()\n    {\n    }\n}",
                (4, 5),
            ),
        ] {
            let d = only(src);
            assert_eq!(d.code(), Code::BraceOnNewLine, "{src}");
            assert_eq!(line_col(src, d.span().start), at, "{src}");
            assert!(
                fix_edits(&d).is_some_and(|(certain, _)| !certain),
                "{src}: {:?}",
                d.fixes()
            );
            joined_parses(src, &d);
        }
    }

    #[test]
    fn brace_on_new_line_has_no_fix() {
        for (src, at) in [
            // The previous statement is not a header: joining gives `print(1) {`.
            ("fn main() {\n    print(1)\n    {\n    }\n}", (3, 5)),
            ("fn main() {\n    x = 1\n    {\n    }\n}", (3, 5)),
            ("fn main() {\n    return 1\n    {\n    }\n}", (3, 5)),
            // A header keyword inside a statement that is not a header.
            ("fn main() {\n    let x = if c\n    {\n    }\n}", (3, 5)),
            (
                "fn main() {\n    x = if c { 1 } else\n    {\n    }\n}",
                (3, 5),
            ),
            // The statement starts after a block's `{`, not at the `if`.
            ("fn main() {\n    if c { x = 1\n    {\n    }\n}}", (3, 5)),
            // The previous line ends with `}`: joining gives `} {`.
            (
                "fn main() {\n    if c { print(1) }\n    {\n    }\n}",
                (3, 5),
            ),
            (
                "fn main() {\n    if true { while false }\n    {\n    }\n}",
                (3, 5),
            ),
            // A `}` after the keyword: the header has its block, or the keyword sits in a closed one.
            (
                "fn main() {\n    if c { 1 } else { 2 } + 3\n    {\n    }\n}",
                (3, 5),
            ),
            (
                "fn main() {\n    if c { while c } == c\n    {\n    }\n}",
                (3, 5),
            ),
            // Inside `(` or `[`, the moved `{` would still sit in the unclosed bracket.
            ("fn main() {\n    if (c\n    { 1 }) {\n    }\n}", (3, 5)),
            (
                "fn main() {\n    for x in [1,\n        2\n    {\n    }]\n}",
                (4, 5),
            ),
        ] {
            let d = only(src);
            assert_eq!(d.code(), Code::BraceOnNewLine, "{src}");
            assert_eq!(line_col(src, d.span().start), at, "{src}");
            assert_eq!(fix_edits(&d), None, "{src}");
        }
    }

    #[test]
    fn certain_brace_fix_removes_the_error() {
        let src = "fn main() {\n    for i in 0..3 // a loop\n    {\n        print(i)\n    }\n}\n";
        let d = only(src);
        let fixed = apply_fix(src, &d);
        assert_eq!(
            fixed,
            "fn main() {\n    for i in 0..3 {\n        print(i)\n    }\n}\n"
        );
        assert!(lex_clean(&fixed).is_ok(), "{fixed}");
    }

    #[test]
    fn semicolon_ends_a_line() {
        use TokenKind::{Eof, Newline, Semicolon};
        assert_eq!(
            all_kinds("x;\ny"),
            [ident("x"), Semicolon, Newline, ident("y"), Newline, Eof]
        );
        // Inside brackets a `;` separates the parts of an array.
        assert_eq!(
            all_kinds("[0;\n3]"),
            [
                TokenKind::LBracket,
                TokenKind::Int(0),
                Semicolon,
                TokenKind::Int(3),
                TokenKind::RBracket,
                Newline,
                Eof
            ]
        );
    }

    #[test]
    fn semicolon_before_a_line_start_lexes_clean() {
        // The `[` after `;` starts an array element, and a `{` after `;` is for the parser.
        for src in [
            "fn main() { let b = [0;\n[2][0]] }",
            "fn main() {\n    let x = 1;\n    {\n    }\n}",
        ] {
            assert!(lex_clean(src).is_ok(), "{src}");
        }
    }

    #[test]
    fn dump_format() {
        let src = "fn main() {}\n";
        assert_eq!(
            dump(src, &tokens(src)),
            "1:1 Fn fn\n1:4 Ident main\n1:8 LParen (\n1:9 RParen )\n1:11 LBrace {\n1:12 RBrace }\n1:13 Newline\n1:13 Eof\n"
        );
    }

    #[test]
    fn reports_every_bad_character() {
        let src = "fn main() {\n    let a = 1 $ 2\n    let b = 3 # 4\n}";
        let lexed = lex(src).expect("the lexer recovers");
        let found: Vec<_> = lexed.diagnostics.iter().map(|d| at(src, d)).collect();
        assert_eq!(
            found,
            [
                (Code::UnexpectedCharacter, 2, 15),
                (Code::UnexpectedCharacter, 3, 15)
            ]
        );
        let kinds: Vec<_> = lexed.tokens.into_iter().map(|t| t.kind).collect();
        assert!(kinds.contains(&TokenKind::Int(2)), "{kinds:?}");
        assert!(kinds.contains(&TokenKind::Int(4)), "{kinds:?}");
    }

    #[test]
    fn bracket_error_yields_to_a_skip() {
        let src = "fn main() {\n    $\n    print((1)\n}";
        assert_eq!(at(src, &fatal(src)), (Code::UnexpectedCharacter, 2, 5));
    }

    #[test]
    fn quote_skip_that_unbalances_reports_the_quote() {
        let src = "fn main() { print(\"abc) }";
        assert_eq!(at(src, &fatal(src)), (Code::UnexpectedCharacter, 1, 19));
    }

    #[test]
    fn bracket_error_alone_is_reported() {
        let d = fatal("(}");
        assert_eq!(d.code(), Code::UnexpectedClosingBracket);
        assert_eq!(d.span(), Span::new(1, 2));
    }

    #[test]
    fn quote_skips_to_the_closing_quote() {
        use TokenKind::{Eof, LParen, Newline, RParen};
        let src = "print(\"a)\")\n";
        let lexed = lex(src).expect("the lexer recovers");
        let found: Vec<_> = lexed.diagnostics.iter().map(|d| at(src, d)).collect();
        assert_eq!(found, [(Code::UnexpectedCharacter, 1, 7)]);
        let kinds: Vec<_> = lexed.tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(kinds, [ident("print"), LParen, RParen, Newline, Eof]);
    }

    #[test]
    fn quote_without_a_close_skips_the_line() {
        use TokenKind::{Eof, Eq, Int, Let, Newline};
        let src = "let n = \"abc\nlet m = 1\n";
        let lexed = lex(src).expect("the lexer recovers");
        assert_eq!(lexed.diagnostics.len(), 1, "{:?}", lexed.diagnostics);
        let kinds: Vec<_> = lexed.tokens.iter().map(|t| t.kind.clone()).collect();
        assert_eq!(
            kinds,
            [
                Let,
                ident("n"),
                Eq,
                Let,
                ident("m"),
                Eq,
                Int(1),
                Newline,
                Eof
            ]
        );
        assert_eq!(line_col(src, lexed.tokens[3].span.start), (2, 1));
    }

    #[test]
    fn quote_at_end_of_file() {
        let src = "fn main() {}\n\"";
        let d = only(src);
        assert_eq!(at(src, &d), (Code::UnexpectedCharacter, 2, 1));
        assert_eq!(d.span(), Span::new(13, 14));
    }

    #[test]
    fn bad_char_at_end_of_file() {
        assert_eq!(err("fn main() {}\n$"), (Code::UnexpectedCharacter, 2, 1));
    }

    #[test]
    fn brace_on_its_own_line_lexes_as_one_line() {
        let lexed = lex("fn main()\n{\n}").expect("the lexer recovers");
        assert_eq!(lexed.diagnostics.len(), 1, "{:?}", lexed.diagnostics);
        let kinds: Vec<_> = lexed.tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(kinds, all_kinds("fn main() {\n}"));
        // After `else` no line break was pushed, so none is dropped.
        let lexed = lex("fn main() { print(if c { 1 } else\n{ 2 }) }").expect("the lexer recovers");
        let kinds: Vec<_> = lexed.tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            all_kinds("fn main() { print(if c { 1 } else { 2 }) }")
        );
    }

    #[test]
    fn bad_literal_lexes_as_zero() {
        use TokenKind::Int;
        let lexed = lex("let a = 1_\nlet b = 2").expect("the lexer recovers");
        assert_eq!(lexed.diagnostics.len(), 1, "{:?}", lexed.diagnostics);
        let ints: Vec<_> = lexed
            .tokens
            .into_iter()
            .map(|t| t.kind)
            .filter(|k| matches!(k, Int(_)))
            .collect();
        assert_eq!(ints, [Int(0), Int(2)]);
    }

    /// The `(code, line, col)` of every diagnostic of `src`, which the lexer recovers from.
    fn found(src: &str) -> Vec<(Code, usize, usize)> {
        let lexed = lex(src).expect("the lexer recovers");
        lexed.diagnostics.iter().map(|d| at(src, d)).collect()
    }

    #[test]
    fn a_skip_damages_its_line() {
        use Code::{
            BraceOnNewLine, BracketOnNewLine, ElseOnNewLine, LeadingBinaryMinus,
            UnexpectedCharacter,
        };
        for (src, want) in [
            // Text skipped on the previous line: the line-start error may be its consequence.
            (
                "fn main() {\n    let a = 1 &\n        - 2\n}",
                vec![(UnexpectedCharacter, 2, 15)],
            ),
            (
                "fn main() {\n    f(a |\n        [0])\n}",
                vec![(UnexpectedCharacter, 2, 9)],
            ),
            (
                "fn main() {\n    if true &\n    {\n    }\n}",
                vec![(UnexpectedCharacter, 2, 13)],
            ),
            (
                "fn main() {\n    let x = 1 \"+\n        - 2\n}",
                vec![(UnexpectedCharacter, 2, 15)],
            ),
            (
                "fn main() {\n    if don$e\n    {\n    }\n}",
                vec![(UnexpectedCharacter, 2, 11)],
            ),
            ("fn main()$\n{\n}", vec![(UnexpectedCharacter, 1, 10)]),
            (
                "fn main() { print(if c { 1 }$\nelse { 2 }) }",
                vec![(UnexpectedCharacter, 1, 29)],
            ),
            // A comment or blank line between does not repair the damaged line.
            (
                "fn main() {\n    let a = 1 $\n    // c\n\n    - 2\n}",
                vec![(UnexpectedCharacter, 2, 15)],
            ),
            // The skip starts the previous line.
            ("let t = a\n$b\n- c", vec![(UnexpectedCharacter, 2, 1)]),
            // Text skipped earlier on the current line.
            (
                "fn main() {\n    print(1)\n    \"if true\" { print(2) }\n}",
                vec![(UnexpectedCharacter, 3, 5)],
            ),
            ("let t = a\n$ - b", vec![(UnexpectedCharacter, 2, 1)]),
            // Independent: the skip is after the token, or before the previous line.
            (
                "let t = a\n- $ b",
                vec![(LeadingBinaryMinus, 2, 1), (UnexpectedCharacter, 2, 3)],
            ),
            (
                "fn main() {\n    let s = 1 $ 2\n    let t = s\n        - 1\n}",
                vec![(UnexpectedCharacter, 2, 15), (LeadingBinaryMinus, 4, 9)],
            ),
            (
                "fn main() {\n    let a = 1$\n    print(a)\n    {\n    }\n}",
                vec![(UnexpectedCharacter, 2, 14), (BraceOnNewLine, 4, 5)],
            ),
            (
                "fn main() {\n    f(a $ b,\n        c\n        [0])\n}",
                vec![(UnexpectedCharacter, 2, 9), (BracketOnNewLine, 4, 9)],
            ),
            (
                "fn main() {\n    print(if c $ {\n        1 }\n    else { 2 })\n}",
                vec![(UnexpectedCharacter, 2, 16), (ElseOnNewLine, 4, 5)],
            ),
        ] {
            assert_eq!(found(src), want, "{src}");
        }
    }

    #[test]
    fn each_brace_fix_holds_alone() {
        // Each certain join, applied alone, leaves the other diagnostics where they were, and the
        // result parses as lexed, with no other join applied.
        for (src, left) in [
            (
                "fn main()\n{\n    if true\n    {\n    }\n}",
                vec![(Code::BraceOnNewLine, 3, 5), (Code::BraceOnNewLine, 2, 1)],
            ),
            // The `$` line is not the one before the `{`.
            (
                "fn f() {\n    let a = 1$\n}\nfn main()\n{\n}",
                vec![(Code::UnexpectedCharacter, 2, 14)],
            ),
        ] {
            let certain: Vec<_> = recovered(src)
                .into_iter()
                .filter(|d| fix_edits(d).is_some_and(|(certain, _)| certain))
                .collect();
            assert_eq!(certain.len(), left.len(), "{src}: {certain:?}");
            for (d, want) in certain.iter().zip(left) {
                let fixed = apply_fix(src, d);
                assert_eq!(found(&fixed), [want], "{fixed}");
                // The lexer's diagnostic stays the only one, and the tokens parse as lexed.
                let lexed = lex(&fixed).expect("the lexer recovers");
                let tokens = lexed.tokens.clone();
                let parsed = crate::parser::parse(lexed);
                let reported: Vec<_> = parsed.diagnostics.iter().map(|d| at(&fixed, d)).collect();
                assert_eq!(reported, [want], "{fixed}");
                let parsed = crate::parser::parse(Lexed {
                    tokens,
                    diagnostics: Vec::new(),
                    comments: Vec::new(),
                    swallowed: Vec::new(),
                });
                assert!(
                    parsed.diagnostics.is_empty() && parsed.checkable,
                    "{fixed}: {:?}",
                    parsed.diagnostics
                );
            }
        }
    }
}
