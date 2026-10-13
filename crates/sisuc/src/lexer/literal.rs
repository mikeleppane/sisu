//! Number literals: decimal, and `0x`, `0b` and `0o` with digit separators.

use super::{Chars, MIN_MAGNITUDE, TokenKind, literal_too_large, take_while};
use crate::diagnostic::{Diagnostic, DiagnosticKind, Span};

/// The literal that starts at the digit `source[start..]`, whose first char `chars` has consumed.
/// A prefixed literal takes every ASCII letter, digit and `_` after its prefix, so a wrong digit
/// is reported at that digit and not as the start of an identifier.
pub(super) fn number(
    source: &str,
    start: usize,
    chars: &mut Chars,
) -> Result<TokenKind, Diagnostic> {
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
    Ok(TokenKind::Int {
        value,
        text: source[start..end].to_string(),
    })
}
