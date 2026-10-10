//! The lexer: source text to tokens.

use std::fmt::Write;
use std::iter::Peekable;
use std::str::CharIndices;

use crate::diagnostic::{Diagnostic, Span, line_col};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Ident(String),
    Int(i64),
    Fn,
    Let,
    Var,
    If,
    Else,
    While,
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
            TokenKind::Var => "Var",
            TokenKind::If => "If",
            TokenKind::Else => "Else",
            TokenKind::While => "While",
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
            TokenKind::Var => "var",
            TokenKind::If => "if",
            TokenKind::Else => "else",
            TokenKind::While => "while",
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

/// An open bracket: its char (`(` or `{`) and where it stands.
type Open = (char, Span);

/// Lexes `source`; the tokens always end with `Eof`. Stops at the first error.
pub(crate) fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    let mut chars = source.char_indices().peekable();
    let mut line_start = true;
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
                TokenKind::Int(int_literal(source, start, end)?)
            }
            '-' if matches!(chars.peek(), Some(&(_, ' ' | '\t')))
                && tokens.last().is_some_and(|t| t.kind == TokenKind::Newline) =>
            {
                return Err(Diagnostic::error(
                    Span::new(start, start + 1),
                    "a line cannot start with a binary `-`",
                )
                .help("to continue the previous line, end it with `-`; to negate, write `-x`"));
            }
            _ => symbol(c, &mut chars).ok_or_else(|| unexpected(c, start))?,
        };
        let end = chars.peek().map_or(source.len(), |&(i, _)| i);
        let span = Span::new(start, end);
        if line_start {
            check_line_start(&tokens, &open, &kind, span)?;
            line_start = false;
        }
        track_bracket(&mut open, &kind, span)?;
        tokens.push(Token { kind, span });
    }
    if let Some(&(c, span)) = open.last() {
        return Err(Diagnostic::error(span, format!("unclosed `{c}`")));
    }
    if ends_statement(&tokens, &open) {
        tokens.push(newline(Span::new(source.len(), source.len())));
    }
    // Eof sits just after the last code, so "found end of file" points at its line.
    let after_code = tokens
        .iter()
        .rfind(|t| t.kind != TokenKind::Newline)
        .map_or(0, |t| t.span.end);
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span::new(after_code, after_code),
    });
    Ok(tokens)
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
                return (code.starts_with('.') || code.starts_with("?."))
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
/// and the innermost open bracket, if any, is a block.
fn ends_statement(tokens: &[Token], open: &[Open]) -> bool {
    can_end_statement(tokens.last()) && open.last().is_none_or(|&(c, _)| c == '{')
}

/// Whether `token` can end a statement (Go's rule).
fn can_end_statement(token: Option<&Token>) -> bool {
    use TokenKind::{
        Break, Continue, False, Ident, Int, NoneKw, Question, RBrace, RParen, Return, SelfKw, True,
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
                | RBrace
        )
    })
}

/// Layout rules 2 and 3 where the parser cannot see them; `kind` starts a line.
/// A `{` only ever follows `fn …`, `if …`, `else` or `while …` on its own line, so a
/// line-initial `{` after a token that could end a statement, or after `else`, breaks
/// rule 2 at any depth. Inside `( )`, where no `Newline` reaches the parser, so does a
/// line-initial `else` (rule 3).
fn check_line_start(
    tokens: &[Token],
    open: &[Open],
    kind: &TokenKind,
    span: Span,
) -> Result<(), Diagnostic> {
    match kind {
        TokenKind::LBrace => {
            let code = tokens.iter().rfind(|t| t.kind != TokenKind::Newline);
            if can_end_statement(code) || code.is_some_and(|t| t.kind == TokenKind::Else) {
                return Err(Diagnostic::error(
                    span,
                    "`{` must be on the same line as `fn`, `if`, `else` or `while`",
                ));
            }
            Ok(())
        }
        TokenKind::Else
            if open.last().is_some_and(|&(c, _)| c == '(') && can_end_statement(tokens.last()) =>
        {
            Err(Diagnostic::error(
                span,
                "`else` must be on the same line as the closing `}`",
            ))
        }
        _ => Ok(()),
    }
}

/// Pushes an opening bracket onto `open`, or checks that a closing one matches the top.
fn track_bracket(open: &mut Vec<Open>, kind: &TokenKind, span: Span) -> Result<(), Diagnostic> {
    let (closes, c) = match kind {
        TokenKind::LParen | TokenKind::LBrace => {
            let c = if *kind == TokenKind::LParen { '(' } else { '{' };
            open.push((c, span));
            return Ok(());
        }
        TokenKind::RParen => ('(', ')'),
        TokenKind::RBrace => ('{', '}'),
        _ => return Ok(()),
    };
    match open.pop() {
        Some((o, _)) if o == closes => Ok(()),
        Some((o, o_span)) => Err(Diagnostic::error(span, format!("unexpected `{c}`"))
            .secondary(o_span, format!("`{o}` opened here"))),
        None => Err(Diagnostic::error(span, format!("unexpected `{c}`"))),
    }
}

/// One token per line, `line:col Kind text`; `Newline` and `Eof` print without text.
pub(crate) fn dump(source: &str, tokens: &[Token]) -> String {
    let mut out = String::new();
    for t in tokens {
        let (line, col) = line_col(source, t.span.start);
        write!(out, "{line}:{col} {}", t.kind.name()).expect("writing to a String cannot fail");
        if !matches!(t.kind, TokenKind::Newline | TokenKind::Eof) {
            write!(out, " {}", &source[t.span.start..t.span.end])
                .expect("writing to a String cannot fail");
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
        "var" => TokenKind::Var,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "while" => TokenKind::While,
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

/// Parses `source[start..end]`, a run of digits and `_`, as an `i64`.
fn int_literal(source: &str, start: usize, end: usize) -> Result<i64, Diagnostic> {
    let text = &source[start..end];
    for (i, _) in text.match_indices('_') {
        let is_digit = |j: usize| text.as_bytes().get(j).is_some_and(u8::is_ascii_digit);
        if !i.checked_sub(1).is_some_and(is_digit) || !is_digit(i + 1) {
            let at = start + i;
            return Err(Diagnostic::error(
                Span::new(at, at + 1),
                "`_` must sit between digits",
            ));
        }
    }
    text.replace('_', "").parse().map_err(|_| {
        Diagnostic::error(
            Span::new(start, end),
            "integer literal is too large for `i64`",
        )
    })
}

/// The operator or punctuation starting with `c`, consuming a second char when it has one.
fn symbol(c: char, chars: &mut Chars) -> Option<TokenKind> {
    use TokenKind::{
        AndAnd, Arrow, Bang, Colon, Comma, Dot, Eq, EqEq, Ge, Gt, LBrace, LParen, Le, Lt, Minus,
        MinusEq, NotEq, OrOr, Percent, PercentEq, Plus, PlusEq, QuestionDot, QuestionQuestion,
        RBrace, RParen, Slash, SlashEq, Star, StarEq,
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
        '.' => Dot,
        '?' if eat('.') => QuestionDot,
        '?' if eat('?') => QuestionQuestion,
        '?' => TokenKind::Question,
        _ => return None,
    })
}

fn unexpected(c: char, at: usize) -> Diagnostic {
    // The span covers the whole char: annotate-snippets panics off a char boundary.
    let d = Diagnostic::error(
        Span::new(at, at + c.len_utf8()),
        format!("unexpected character `{c}`"),
    );
    match c {
        '&' => d.help("use `&&` for logical and"),
        '|' => d.help("use `||` for logical or"),
        _ => d,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::{Span, line_col};

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src)
            .expect("source lexes")
            .into_iter()
            .map(|t| t.kind)
            .filter(|k| *k != TokenKind::Newline)
            .collect()
    }

    fn err(src: &str) -> (String, usize, usize) {
        let d = lex(src).expect_err("source fails to lex");
        let (line, col) = line_col(src, d.span.start);
        (d.message, line, col)
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
        use TokenKind::{Else, Eof, False, Fn, If, Let, Return, True, Var, While};
        assert_eq!(
            kinds("fn let var if else while return true false"),
            [Fn, Let, Var, If, Else, While, Return, True, False, Eof]
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
            Class, Colon, Dot, Eof, Is, Let, NoneKw, Question, QuestionDot, QuestionQuestion,
            SelfKw, Var,
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
            all_kinds("var next: Tree?\nlet"),
            [
                Var,
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
        assert_eq!(
            err("1__0"),
            ("`_` must sit between digits".to_string(), 1, 2)
        );
    }

    #[test]
    fn trailing_underscore() {
        assert_eq!(
            err("100_"),
            ("`_` must sit between digits".to_string(), 1, 4)
        );
    }

    #[test]
    fn max_literal() {
        assert_eq!(
            kinds("9223372036854775807"),
            [TokenKind::Int(i64::MAX), TokenKind::Eof]
        );
    }

    #[test]
    fn literal_too_large() {
        assert_eq!(
            err("9223372036854775808"),
            ("integer literal is too large for `i64`".to_string(), 1, 1)
        );
    }

    #[test]
    fn lone_ampersand() {
        let d = lex("a & b").expect_err("lone `&`");
        assert_eq!(d.message, "unexpected character `&`");
        assert_eq!(d.help.as_deref(), Some("use `&&` for logical and"));
        assert_eq!(line_col("a & b", d.span.start), (1, 3));
    }

    #[test]
    fn lone_pipe() {
        let d = lex("a | b").expect_err("lone `|`");
        assert_eq!(d.message, "unexpected character `|`");
        assert_eq!(d.help.as_deref(), Some("use `||` for logical or"));
    }

    #[test]
    fn unknown_char_counts_chars() {
        let src = "// ä\nlet é = 1";
        let d = lex(src).expect_err("`é` is not a Sisu character");
        assert_eq!(d.message, "unexpected character `é`");
        assert_eq!(line_col(src, d.span.start), (2, 5));
        // The span covers the whole two-byte char, so rendering does not panic.
        assert!(d.render("t.sisu", src).contains("2:5"));
    }

    #[test]
    fn comment_skipped() {
        assert_eq!(kinds("x // päivää"), [ident("x"), TokenKind::Eof]);
    }

    #[test]
    fn token_spans_are_byte_offsets() {
        // The two-byte `ä` shifts byte offsets away from char offsets.
        let spans: Vec<Span> = lex("// ä\nab 12 +=")
            .expect("source lexes")
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
        let spans = |src| -> Vec<Span> {
            lex(src)
                .expect("source lexes")
                .into_iter()
                .map(|t| t.span)
                .collect()
        };
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
        let d = lex(src).expect_err("U+00A0 is not Sisu whitespace");
        assert_eq!(d.message, "unexpected character `\u{00a0}`");
        assert_eq!(line_col(src, d.span.start), (2, 1));
    }

    fn all_kinds(src: &str) -> Vec<TokenKind> {
        lex(src)
            .expect("source lexes")
            .into_iter()
            .map(|t| t.kind)
            .collect()
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
        let spans = |src| -> Vec<Span> {
            lex(src)
                .expect("source lexes")
                .into_iter()
                .map(|t| t.span)
                .collect()
        };
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
        let src = "let t = a\n- b";
        let d = lex(src).expect_err("a line cannot start with a binary `-`");
        assert_eq!(d.message, "a line cannot start with a binary `-`");
        assert_eq!(line_col(src, d.span.start), (2, 1));
        assert_eq!(
            d.help.as_deref(),
            Some("to continue the previous line, end it with `-`; to negate, write `-x`")
        );
    }

    #[test]
    fn unexpected_close() {
        assert_eq!(err("x)"), ("unexpected `)`".to_string(), 1, 2));
    }

    #[test]
    fn mismatched_close() {
        let d = lex("(}").expect_err("`}` does not close `(`");
        assert_eq!(d.message, "unexpected `}`");
        assert_eq!(line_col("(}", d.span.start), (1, 2));
        let [(open, text)] = d.secondary.as_slice() else {
            panic!("one secondary span, got {:?}", d.secondary);
        };
        assert_eq!(text, "`(` opened here");
        assert_eq!(line_col("(}", open.start), (1, 1));
    }

    #[test]
    fn unclosed_paren() {
        assert_eq!(err("(x"), ("unclosed `(`".to_string(), 1, 1));
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
    fn else_on_new_line_inside_parens() {
        assert_eq!(
            err("fn main() { print(if true { 1 }\nelse { 2 }) }"),
            (
                "`else` must be on the same line as the closing `}`".to_string(),
                2,
                1
            )
        );
    }

    const BRACE_ON_NEW_LINE: &str = "`{` must be on the same line as `fn`, `if`, `else` or `while`";

    #[test]
    fn brace_on_new_line_inside_parens() {
        assert_eq!(
            err("fn main() { print(if true\n{ 1 } else { 2 }) }"),
            (BRACE_ON_NEW_LINE.to_string(), 2, 1)
        );
    }

    #[test]
    fn brace_on_new_line_after_else() {
        assert_eq!(
            err("fn main() {\n    if true { print(1) } else\n    { print(2) }\n}"),
            (BRACE_ON_NEW_LINE.to_string(), 3, 5)
        );
    }

    #[test]
    fn brace_on_new_line_after_else_inside_parens() {
        assert_eq!(
            err("fn main() { print(if true { 1 } else\n{ 2 }) }"),
            (BRACE_ON_NEW_LINE.to_string(), 2, 1)
        );
    }

    #[test]
    fn brace_on_new_line_after_fn() {
        assert_eq!(
            err("fn main()\n{\n}"),
            (BRACE_ON_NEW_LINE.to_string(), 2, 1)
        );
    }

    #[test]
    fn dump_format() {
        let src = "fn main() {}\n";
        assert_eq!(
            dump(src, &lex(src).expect("source lexes")),
            "1:1 Fn fn\n1:4 Ident main\n1:8 LParen (\n1:9 RParen )\n1:11 LBrace {\n1:12 RBrace }\n1:13 Newline\n1:13 Eof\n"
        );
    }
}
