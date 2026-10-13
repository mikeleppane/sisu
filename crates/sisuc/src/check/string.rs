//! Conversions between the scalar types: `i64(x)`, `u8(x)` and `char(x)`.

use super::expr::{path, typed};
use super::{Checked, Checker, Poisoned, type_of};
use crate::ast::{self, ExprKind};
use crate::diagnostic::{DiagnosticKind, Span};
use crate::tir::{self, Type};

impl Checker {
    /// `to(arg)`: `i64(x)` takes a `u8` or a `char`; `u8(x)` and `char(x)` take an `i64`. Its
    /// argument expects no type, so a literal one is an `i64`.
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
        let from: &[Type] = if to == Type::I64 {
            &[Type::U8, Type::Char]
        } else {
            &[Type::I64]
        };
        let checked = self.value(&arg.value, None);
        let d = match type_of(&checked) {
            Some(ty) if *ty == to => {
                let grouped = !single_term(&arg.value);
                // The removal deletes the head, and the tail unless the value stays grouped.
                let head = span.start..arg.value.span.start;
                let tail = arg.value.span.end..span.end;
                let commented = self
                    .comments
                    .iter()
                    .any(|c| head.contains(c) || (!grouped && tail.contains(c)));
                DiagnosticKind::AlreadyConverted {
                    value: path(&arg.value),
                    ty: self.type_name(&to),
                    at: arg.value.span,
                    conversion: span,
                    grouped,
                    labeled,
                    commented,
                }
                .into()
            }
            // The one type `x` should have is offered a conversion to it.
            Some(ty) if !from.contains(ty) => match from {
                [only] => self.mismatch(arg.value.span, Some(&arg.value), only, ty),
                _ => DiagnosticKind::Mismatch {
                    want: from.iter().map(|ty| self.type_name(ty)).collect(),
                    got: self.type_name(ty),
                    at: arg.value.span,
                    convert: None,
                }
                .into(),
            },
            _ => {
                let kind = checked
                    .ok()
                    .filter(|_| !labeled)
                    .map(|arg| tir::ExprKind::Convert(Box::new(arg)));
                return typed(span, to, labeled, kind);
            }
        };
        self.diagnostics.push(d);
        Err(Poisoned::error())
    }
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
