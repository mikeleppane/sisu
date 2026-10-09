//! The lexer: source text to tokens.

use std::iter::Peekable;
use std::str::CharIndices;

use crate::diagnostic::{Diagnostic, Span};

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
    Newline,
    Eof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) span: Span,
}

type Chars<'a> = Peekable<CharIndices<'a>>;

/// Lexes `source`; the tokens always end with `Eof`. Stops at the first error.
pub(crate) fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    let mut tokens = Vec::new();
    let mut chars = source.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let kind = match c {
            // Task 3 turns '\n' into a Newline token.
            ' ' | '\t' | '\r' | '\n' => continue,
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
            _ => symbol(c, &mut chars).ok_or_else(|| unexpected(c, start))?,
        };
        let end = chars.peek().map_or(source.len(), |&(i, _)| i);
        tokens.push(Token {
            kind,
            span: Span::new(start, end),
        });
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span::new(source.len(), source.len()),
    });
    Ok(tokens)
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
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        _ => TokenKind::Ident(text.to_string()),
    }
}

/// Parses `source[start..end]`, a run of digits and `_`, as an `i64`.
fn int_literal(source: &str, start: usize, end: usize) -> Result<i64, Diagnostic> {
    let text = &source[start..end];
    for (i, _) in text.match_indices('_') {
        let digit_at = |j: Option<usize>| j.is_some_and(|j| text.as_bytes()[j].is_ascii_digit());
        if !digit_at(i.checked_sub(1)) || !digit_at(Some(i + 1).filter(|&j| j < text.len())) {
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
        AndAnd, Arrow, Bang, Colon, Comma, Eq, EqEq, Ge, Gt, LBrace, LParen, Le, Lt, Minus,
        MinusEq, NotEq, OrOr, Percent, PercentEq, Plus, PlusEq, RBrace, RParen, Slash, SlashEq,
        Star, StarEq,
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
        let spans: Vec<Span> = lex("ab 12 +=")
            .expect("source lexes")
            .into_iter()
            .map(|t| t.span)
            .collect();
        assert_eq!(
            spans,
            [
                Span::new(0, 2),
                Span::new(3, 5),
                Span::new(6, 8),
                Span::new(8, 8)
            ]
        );
    }
}
