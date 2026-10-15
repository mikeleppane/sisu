//! Literals: numbers (decimal, and `0x`, `0b` and `0o` with digit separators), `char` and
//! string literals, and the escapes in them.

use super::{Chars, MIN_MAGNITUDE, TokenKind, literal_too_large, take_while};
use crate::diagnostic::{Diagnostic, DiagnosticKind, LiteralKind, Span};

/// The number or `char` literal that starts at `source[start..]`, whose first char `chars` has
/// consumed. After an error it records the diagnostic and gives a placeholder token, so that
/// lexing and parsing go on.
pub(super) fn token(
    source: &str,
    start: usize,
    chars: &mut Chars,
    diagnostics: &mut Vec<Diagnostic>,
) -> TokenKind {
    let (literal, placeholder) = if source[start..].starts_with('\'') {
        (char_literal(source, start, chars), TokenKind::Char('\0'))
    } else {
        (number(source, start, chars), TokenKind::Int(0))
    };
    literal.unwrap_or_else(|d| {
        diagnostics.push(d);
        placeholder
    })
}

/// The literal that starts at the digit `source[start..]`, whose first char `chars` has consumed.
/// A prefixed literal takes every ASCII letter, digit and `_` after its prefix, so a wrong digit
/// is reported at that digit and not as the start of an identifier.
fn number(source: &str, start: usize, chars: &mut Chars) -> Result<TokenKind, Diagnostic> {
    let prefix = if source[start..].starts_with('0') {
        chars.next_if(|&(_, c)| matches!(c, 'x' | 'X' | 'b' | 'B' | 'o' | 'O'))
    } else {
        None
    };
    let (radix, digits_at, end) = match prefix {
        Some((_, p)) => {
            let radix = match p.to_ascii_lowercase() {
                'x' => 16,
                'b' => 2,
                _ => 8,
            };
            let end = take_while(chars, source, |c| c.is_ascii_alphanumeric() || c == '_');
            (radix, start + 2, end)
        }
        None => (
            10,
            start,
            take_while(chars, source, |c| c.is_ascii_digit() || c == '_'),
        ),
    };
    let whole = Span::new(start, end);
    let digits = &source[digits_at..end];
    if let Some((_, prefix)) = prefix.filter(|_| digits.is_empty()) {
        return Err(DiagnosticKind::BasePrefixWithoutDigits { prefix, at: whole }.into());
    }
    for (i, c) in digits.char_indices() {
        let at = digits_at + i;
        let neighbor = |j: Option<usize>| {
            j.and_then(|j| digits.as_bytes().get(j))
                .is_some_and(u8::is_ascii_alphanumeric)
        };
        if c == '_' {
            if !neighbor(i.checked_sub(1)) || !neighbor(Some(i + 1)) {
                return Err(DiagnosticKind::DigitSeparator {
                    at: Span::new(at, at + 1),
                }
                .into());
            }
        } else if !c.is_digit(radix) {
            return Err(DiagnosticKind::DigitOutsideBase {
                digit: c,
                radix,
                at: Span::new(at, at + 1),
            }
            .into());
        }
    }
    let value = u64::from_str_radix(&digits.replace('_', ""), radix)
        .ok()
        .filter(|&n| n <= MIN_MAGNITUDE)
        .ok_or_else(|| literal_too_large(whole))?;
    // Reported last, so that its fix leaves a literal that lexes. A bare 2^63 still fails to
    // parse; only a `-` before it makes it valid, which the lexer cannot see.
    if let Some((_, p)) = prefix.filter(|(_, p)| p.is_ascii_uppercase()) {
        let at = Span::new(start, start + 2);
        let certain = value != MIN_MAGNITUDE;
        return Err(DiagnosticKind::BasePrefixUpperCase {
            prefix: p,
            certain,
            at,
        }
        .into());
    }
    Ok(TokenKind::Int(value))
}

/// The `char` literal that starts at the `'` at `source[start..]`, which `chars` has consumed.
/// It reads on to the closing `'`, so that an error leaves the rest of the literal behind it, or
/// to the line end when there is none. The first error is the one reported.
fn char_literal(source: &str, start: usize, chars: &mut Chars) -> Result<TokenKind, Diagnostic> {
    let mut error = None;
    let mut points = String::new();
    let end = loop {
        let Some((i, c)) = chars.next_if(|&(_, c)| c != '\n') else {
            return Err(error.unwrap_or_else(|| {
                DiagnosticKind::LiteralNotClosed {
                    kind: LiteralKind::Char,
                    at: Span::new(start, start + 1),
                }
                .into()
            }));
        };
        match c {
            '\'' => break i + 1,
            // A `\` at the line end is followed by the "not closed" error of a later turn.
            '\\' if line_ends(source, chars) => {}
            '\\' => match escape(source, i, chars) {
                Ok(c) => points.push(c),
                Err(d) => {
                    error.get_or_insert(d);
                }
            },
            c => points.push(c),
        }
    };
    if let Some(d) = error {
        return Err(d);
    }
    let mut points = points.chars();
    match (points.next(), points.next()) {
        (Some(c), None) => Ok(TokenKind::Char(c)),
        _ => Err(DiagnosticKind::NotOneCodePoint {
            at: Span::new(start, end),
            string: !source[start..end].contains(['"', '{', '}']),
        }
        .into()),
    }
}

/// How a piece of a string literal's text ends.
pub(super) enum End {
    /// At the closing `"`.
    Quote,
    /// At the `{` of an interpolation.
    Brace,
    /// At a line break or the end of the file.
    Open,
}

/// The text of a string literal after its `"`, or after the `}` that ends an interpolation,
/// which `chars` has consumed: up to and with its closing `"` or the `{` of an interpolation, or
/// up to the line end, which the caller reports. The first error is the one reported.
pub(super) fn string_text(
    source: &str,
    chars: &mut Chars,
    diagnostics: &mut Vec<Diagnostic>,
) -> (String, End) {
    let mut error = None;
    let mut text = String::new();
    let end = loop {
        let Some((i, c)) = chars.next_if(|&(_, c)| c != '\n') else {
            break End::Open;
        };
        match c {
            '"' => break End::Quote,
            '{' => {
                let inside = &source[i + 1..];
                // What the main loop skips.
                let close = source.len() - inside.trim_start_matches([' ', '\t', '\r']).len();
                if !source[close..].starts_with('}') {
                    break End::Brace;
                }
                while chars.next_if(|&(j, _)| j <= close).is_some() {}
                error.get_or_insert_with(|| {
                    DiagnosticKind::EmptyInterpolation {
                        at: Span::new(i, close + 1),
                    }
                    .into()
                });
            }
            '}' => {
                error.get_or_insert_with(|| {
                    DiagnosticKind::BraceInString {
                        at: Span::new(i, i + 1),
                    }
                    .into()
                });
            }
            // A `\` at the line end is followed by the "not closed" error of a later turn.
            '\\' if line_ends(source, chars) => {}
            '\\' => match escape(source, i, chars) {
                Ok(c) => text.push(c),
                Err(d) => {
                    error.get_or_insert(d);
                }
            },
            c => text.push(c),
        }
    };
    diagnostics.extend(error);
    (text, end)
}

/// The error for a string literal whose `"` stands at `quote` and that a line break or the end
/// of the file stops.
pub(super) fn not_closed(quote: usize) -> Diagnostic {
    DiagnosticKind::LiteralNotClosed {
        kind: LiteralKind::String,
        at: Span::new(quote, quote + 1),
    }
    .into()
}

/// Whether the line or the file ends at the next char of `chars`, where `\r\n` ends a line too.
fn line_ends(source: &str, chars: &mut Chars) -> bool {
    chars
        .peek()
        .is_none_or(|&(i, _)| source[i..].starts_with('\n') || source[i..].starts_with("\r\n"))
}

/// Decodes the escape after a `\`, which `chars` has just consumed at `backslash`. A char that is
/// not a line break must follow: the caller reports a literal that ends there as not closed.
pub(super) fn escape(
    source: &str,
    backslash: usize,
    chars: &mut Chars,
) -> Result<char, Diagnostic> {
    let Some((at, c)) = chars.next() else {
        unreachable!("the caller checked that a char follows the backslash")
    };
    match c {
        'n' => Ok('\n'),
        't' => Ok('\t'),
        'r' => Ok('\r'),
        '0' => Ok('\0'),
        '\\' | '"' | '\'' | '{' | '}' => Ok(c),
        'u' => unicode_escape(source, backslash, chars),
        _ => Err(DiagnosticKind::UnknownEscape {
            found: c,
            at: Span::new(backslash, at + c.len_utf8()),
        }
        .into()),
    }
}

/// The value of `\u{...}`, whose `\` stands at `backslash` and whose `u` `chars` has consumed.
fn unicode_escape(source: &str, backslash: usize, chars: &mut Chars) -> Result<char, Diagnostic> {
    let malformed = |end| -> Diagnostic {
        DiagnosticKind::MalformedUnicodeEscape {
            at: Span::new(backslash, end),
        }
        .into()
    };
    let digits_at = backslash + 3;
    if chars.next_if(|&(_, c)| c == '{').is_none() {
        return Err(malformed(backslash + 2));
    }
    let digits_end = take_while(chars, source, |c| c.is_ascii_hexdigit());
    if chars.next_if(|&(_, c)| c == '}').is_none() {
        return Err(malformed(digits_end));
    }
    let end = digits_end + 1;
    let digits = &source[digits_at..digits_end];
    if digits.is_empty() || digits.len() > 6 {
        return Err(malformed(end));
    }
    u32::from_str_radix(digits, 16)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| {
            DiagnosticKind::NotScalarValue {
                escape: source[backslash..end].to_string(),
                at: Span::new(backslash, end),
            }
            .into()
        })
}
