//! `string`: interpolation, `+` that joins two, a chain of comparisons over them, `len()`,
//! `chars()` and `bytes()`, and the guess to interpolate a value that `+` or `string(x)` meets.
//! Conversions: `i64(x)`, `u8(x)` and `char(x)` between the scalar types, and `string(cs)` from
//! a `[char]`.

use std::fmt::Write;

use super::array::pair;
use super::expr::{local, no_member, path, typed};
use super::{Checked, Checker, Poisoned, block_expr, payload, type_of};
use crate::ast::{self, CompareOp, ExprKind};
use crate::diagnostic::{
    Definition, Diagnostic, DiagnosticKind, InterpolateHelp, MismatchFix, Operand, Span,
};
use crate::tir::{self, Type};

impl Checker {
    /// `l + r` over two `string`s, which `mismatched` says were reported against.
    pub(super) fn concat(
        &mut self,
        span: Span,
        l: Checked,
        r: Checked,
        mismatched: bool,
    ) -> Checked {
        // A join with an `Error` operand is `Error`: that operand may have been meant as a
        // number, and a `char` or an expected type beside the join would report it again.
        let known = !mismatched && type_of(&l).is_some() && type_of(&r).is_some();
        let (Ok(l), Ok(r)) = (l, r) else {
            return Err(Poisoned::of(known.then_some(Type::String)));
        };
        Ok(self.spilled(span, Type::String, vec![l, r], |operands| {
            let (lhs, rhs) = pair(operands);
            tir::ExprKind::Concat { lhs, rhs }
        }))
    }

    /// A string literal with interpolations. Each part is checked with no expected type, so that
    /// a `unit` one is reported here, and must be of a type that interpolation writes as text. A
    /// part that never finishes makes the whole never finish.
    pub(super) fn interpolate(&mut self, span: Span, parts: &[ast::Expr]) -> Checked {
        let mut ty = Type::String;
        let mut checked = Vec::with_capacity(parts.len());
        for part in parts {
            let value = self.expr(part, None);
            let help = match type_of(&value) {
                None | Some(Type::I64 | Type::U8 | Type::Bool | Type::Char | Type::String) => {
                    checked.push(value);
                    continue;
                }
                Some(Type::Never) => {
                    ty = Type::Never;
                    checked.push(value);
                    continue;
                }
                Some(other) if matches!(payload(other), Type::Array(_)) => {
                    Some(InterpolateHelp::ForLoop)
                }
                Some(Type::Optional(_)) => Some(InterpolateHelp::Unwrap),
                Some(Type::Unit | Type::Class(_) | Type::Array(_)) => None,
            };
            let found = type_of(&value).expect("a part reported here should have a type");
            self.report(DiagnosticKind::CannotInterpolate {
                ty: self.type_name(found),
                help,
                at: part.span,
            });
            checked.push(Err(Poisoned::error()));
        }
        match checked.into_iter().collect::<Result<Vec<_>, _>>() {
            Ok(parts) => Ok(self.spilled(span, ty, parts, tir::ExprKind::Interpolate)),
            Err(_) => Err(Poisoned::typed(ty)),
        }
    }

    /// The operand `e` of the `+` over `operands`, of type `got`, which is not the `string` the
    /// other operand makes it; that operand gets a secondary label. A value that interpolation prints gets the guess to interpolate
    /// it, unless the `+` is a `compound` assignment's or a constant's, a comment between the
    /// operands would go with the text the guess removes, or an operand spans a line break,
    /// which a string literal cannot hold.
    pub(super) fn concat_mismatch(
        &self,
        e: &ast::Expr,
        got: &Type,
        operands: &[&ast::Expr],
        compound: bool,
    ) -> Diagnostic {
        let printed = matches!(
            got,
            Type::I64 | Type::U8 | Type::Bool | Type::Char | Type::String
        );
        let [lhs, rhs] = operands else {
            unreachable!("`+` should have two operands");
        };
        let between = lhs.span.end..rhs.span.start;
        let commented = self.comments.iter().any(|c| between.contains(c));
        let lines = operands.iter().any(|e| !self.one_line(e.span));
        // A literal in parentheses starts at its `(`, and is wrapped whole like any other value.
        let operand = |e: &ast::Expr| Operand {
            span: e.span,
            literal: matches!(e.kind, ExprKind::Str(_) | ExprKind::Interpolate(_))
                && self.quotes.binary_search(&e.span.start).is_ok(),
        };
        let guess = !(compound || self.in_constant || !printed || commented || lines);
        let other = if e.span == lhs.span {
            rhs.span
        } else {
            lhs.span
        };
        DiagnosticKind::Mismatch {
            want: vec![self.type_name(&Type::String)],
            got: self.type_name(got),
            at: e.span,
            other: Some(other),
            fix: guess.then(|| MismatchFix::Interpolate {
                lhs: operand(lhs),
                rhs: operand(rhs),
            }),
        }
        .into()
    }

    /// Whether `span` holds no line break, which a string literal cannot hold. A comment inside
    /// it would end at one, so it holds no comment either, and fits inside `{}`.
    pub(super) fn one_line(&self, span: Span) -> bool {
        let next = self.breaks.partition_point(|&b| b < span.start);
        self.breaks.get(next).is_none_or(|&b| b >= span.end)
    }

    /// A chain over the `string`s `operands`, which codegen compares two at a time. Two make
    /// one `Compare`; more bind each operand once, in nested blocks that stop at the first pair
    /// that fails. `a < b < c` is
    /// `(block (let #0 a) (let #1 b) (&& (< #0 #1) (block (let #2 c) (< #1 #2))))`.
    pub(super) fn string_chain(
        &mut self,
        span: Span,
        operands: Vec<tir::Expr>,
        ops: &[CompareOp],
    ) -> tir::Expr {
        if let [op] = ops {
            return self.spilled(span, Type::Bool, operands, |operands| {
                tir::ExprKind::Compare {
                    operands,
                    ops: vec![*op],
                }
            });
        }
        let mut lets = Vec::new();
        let reads: Vec<_> = operands
            .into_iter()
            .map(|init| {
                let read = (self.fresh(Type::String), init.span);
                lets.push(tir::Stmt::Let {
                    local: read.0,
                    init,
                });
                read
            })
            .collect();
        let read = |i: usize| local(reads[i].0, Type::String, reads[i].1);
        let pair = |i: usize| tir::Expr {
            kind: tir::ExprKind::Compare {
                operands: vec![read(i), read(i + 1)],
                ops: vec![ops[i]],
            },
            ty: Type::Bool,
            span,
        };
        // From the innermost block out: each binds the right operand of the pair it tests.
        let mut rest = pair(ops.len() - 1);
        for i in (1..ops.len()).rev() {
            let bind = lets.pop().expect("each operand should have its `let`");
            let inner = block_expr(vec![bind], Some(rest), Type::Bool, span);
            rest = and(pair(i - 1), inner, span);
        }
        block_expr(lets, Some(rest), Type::Bool, span)
    }

    /// `receiver.method(args)` on a `string`: `len()`, `chars()` and `bytes()`.
    pub(super) fn string_method(
        &mut self,
        span: Span,
        receiver: Checked,
        method: &ast::Ident,
        args: &[ast::Arg],
    ) -> Checked {
        let (ty, node): (_, fn(Box<tir::Expr>) -> tir::ExprKind) = match method.name.as_str() {
            "len" => (Type::I64, tir::ExprKind::Len),
            "chars" => (Type::Array(Box::new(Type::Char)), tir::ExprKind::Chars),
            "bytes" => (Type::Array(Box::new(Type::U8)), tir::ExprKind::Bytes),
            _ => {
                let owner = self.type_name(&Type::String);
                let d = no_member(Definition::Method, owner, method, ["len", "chars", "bytes"]);
                return self.rejected(d, args);
            }
        };
        if !args.is_empty() {
            let d = DiagnosticKind::ArgumentCount {
                name: method.name.to_string(),
                want: 0,
                found: args.len(),
                at: span,
            };
            return self.rejected(d.into(), args);
        }
        let kind = receiver.ok().map(|r| node(Box::new(r)));
        typed(span, ty, false, kind)
    }

    /// `to(arg)`: `i64(x)` takes a `u8` or a `char`; `u8(x)` and `char(x)` take an `i64`; and
    /// `string(x)` takes a `[char]`. The argument of a scalar conversion expects no type, so a
    /// literal one is an `i64`; `string([])` is the empty string.
    pub(super) fn convert(&mut self, span: Span, to: Type, args: &[ast::Arg]) -> Checked {
        let labeled = self.labels(args);
        let [arg] = args else {
            self.report(DiagnosticKind::ArgumentCount {
                name: to.to_string(),
                want: 1,
                found: args.len(),
                at: span,
            });
            self.stray(args);
            return Err(Poisoned::error());
        };
        let chars = [Type::Array(Box::new(Type::Char))];
        let from: &[Type] = match to {
            Type::I64 => &[Type::U8, Type::Char],
            Type::String => &chars,
            _ => &[Type::I64],
        };
        let checked = self.value(&arg.value, (to == Type::String).then_some(&chars[0]));
        // The head, `to(`, and the tail, `)`, are the text a rewrite of the call deletes.
        let head = span.start..arg.value.span.start;
        let tail = arg.value.span.end..span.end;
        let d = match type_of(&checked) {
            Some(ty) if *ty == to => {
                let grouped = !single_term(&arg.value);
                // The removal deletes the head, and the tail unless the value stays grouped.
                let commented = self
                    .comments
                    .iter()
                    .any(|c| head.contains(c) || (!grouped && tail.contains(c)));
                DiagnosticKind::AlreadyConverted {
                    value: path(&arg.value).or_else(|| literal(&arg.value)),
                    ty: self.type_name(&to),
                    at: arg.value.span,
                    conversion: span,
                    grouped,
                    labeled,
                    commented,
                }
                .into()
            }
            // The one type `x` should have is offered a conversion to it, or for a `string`,
            // interpolation of the value.
            Some(ty) if !from.contains(ty) => match from {
                [only] if to == Type::String => {
                    self.string_call_mismatch(span, &arg.value, ty, only, &[head, tail])
                }
                [only] => self.mismatch(arg.value.span, Some(&arg.value), only, ty),
                _ => DiagnosticKind::Mismatch {
                    want: from.iter().map(|ty| self.type_name(ty)).collect(),
                    got: self.type_name(ty),
                    at: arg.value.span,
                    other: None,
                    fix: None,
                }
                .into(),
            },
            _ => {
                let node = if to == Type::String {
                    tir::ExprKind::FromChars
                } else {
                    tir::ExprKind::Convert
                };
                let kind = checked
                    .ok()
                    .filter(|_| !labeled)
                    .map(|arg| node(Box::new(arg)));
                return typed(span, to, labeled, kind);
            }
        };
        self.diagnostics.push(d);
        Err(Poisoned::error())
    }

    /// `string(arg)` where `arg`, of type `got`, is not a `want`. A value that interpolation
    /// prints is offered `"{arg}"` instead, unless a comment sits in `deleted`, the text the
    /// rewrite replaces, or the argument spans a line break, which a string literal cannot hold.
    fn string_call_mismatch(
        &self,
        call: Span,
        arg: &ast::Expr,
        got: &Type,
        want: &Type,
        deleted: &[std::ops::Range<usize>],
    ) -> Diagnostic {
        let printed = matches!(got, Type::I64 | Type::U8 | Type::Bool | Type::Char);
        let commented = self
            .comments
            .iter()
            .any(|c| deleted.iter().any(|d| d.contains(c)));
        let next = self.breaks.partition_point(|&b| b < arg.span.start);
        let lines = self.breaks.get(next).is_some_and(|&b| b < arg.span.end);
        if !printed || commented || lines {
            return self.mismatch(arg.span, Some(arg), want, got);
        }
        DiagnosticKind::Mismatch {
            want: vec![self.type_name(want)],
            got: self.type_name(got),
            at: arg.span,
            other: None,
            fix: Some(MismatchFix::InterpolateCall {
                call,
                arg: arg.span,
            }),
        }
        .into()
    }
}

/// A string literal, for the message, in Sisu's escapes: no other literal is named.
fn literal(e: &ast::Expr) -> Option<String> {
    let ExprKind::Str(text) = &e.kind else {
        return None;
    };
    let mut quoted = String::from('"');
    for c in text.chars() {
        match c {
            '\\' | '"' | '{' | '}' => quoted.extend(['\\', c]),
            '\n' => quoted.push_str("\\n"),
            '\t' => quoted.push_str("\\t"),
            '\r' => quoted.push_str("\\r"),
            '\0' => quoted.push_str("\\0"),
            c if c.is_control() => write!(quoted, "\\u{{{:X}}}", u32::from(c))
                .expect("writing to a `String` should never fail"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    Some(quoted)
}

/// Whether `e` is one term as written: a name, a literal, a call, a field or index read, or an
/// expression in parentheses. Anything else needs its parentheses once it sits beside other code.
/// A parenthesized node is the only one whose span reaches past its parts.
fn single_term(e: &ast::Expr) -> bool {
    match &e.kind {
        ExprKind::Int { .. }
        | ExprKind::Bool(_)
        | ExprKind::Char(_)
        | ExprKind::Str(_)
        | ExprKind::Interpolate(_)
        | ExprKind::Name(_)
        | ExprKind::SelfValue
        | ExprKind::NoneLit
        | ExprKind::Call { .. }
        | ExprKind::Todo(_)
        | ExprKind::Field { .. }
        | ExprKind::MethodCall { .. }
        | ExprKind::Index { .. }
        | ExprKind::ArrayLit(_)
        | ExprKind::ArrayFill { .. } => true,
        ExprKind::Binary { lhs, rhs, .. } | ExprKind::Coalesce { lhs, rhs } => {
            e.span != lhs.span.to(rhs.span)
        }
        // A `bool`, which no conversion holds: never reached, and grouping is always right.
        ExprKind::Is { .. } | ExprKind::Compare { .. } => false,
        ExprKind::Unary { operand, .. } => e.span.end != operand.span.end,
        ExprKind::Block(block) => e.span != block.span,
        ExprKind::If {
            then_block,
            else_block,
            ..
        }
        | ExprKind::IfLet {
            then_block,
            else_block,
            ..
        } => e.span.end != else_block.as_ref().unwrap_or(then_block).span.end,
    }
}

/// `lhs && rhs`, at `span`.
fn and(lhs: tir::Expr, rhs: tir::Expr, span: Span) -> tir::Expr {
    tir::Expr {
        kind: tir::ExprKind::Binary {
            op: tir::BinaryOp::And,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        ty: Type::Bool,
        span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{TokenKind, lex};

    /// The name of a literal is Sisu that lexes back to the literal's text.
    #[test]
    fn literal_names_lex_back() {
        for text in [
            "hi!",
            "{}",
            "\\\"",
            "a\nb\tc\rd\0e",
            "\u{7}\u{1B}\u{7F}\u{85}",
            "é😀",
        ] {
            let e = ast::Expr {
                kind: ExprKind::Str(text.to_string()),
                span: Span::new(0, 0),
            };
            let name = literal(&e).expect("a string literal has a name");
            let lexed = lex(&name).expect("the name should lex");
            assert_eq!(
                lexed.tokens[0].kind,
                TokenKind::Str(text.to_string()),
                "{name}"
            );
            assert!(lexed.diagnostics.is_empty(), "{name}");
        }
    }
}
