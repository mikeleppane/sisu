//! `string`: `+` that joins two, a chain of comparisons over them, `len()`, and the guess to
//! interpolate a value that `+` meets beside one. Conversions between the scalar types:
//! `i64(x)`, `u8(x)` and `char(x)`.

use super::array::pair;
use super::expr::{local, no_member, path, typed};
use super::{Checked, Checker, Poisoned, block_expr, type_of};
use crate::ast::{self, CompareOp, ExprKind};
use crate::diagnostic::{Definition, Diagnostic, DiagnosticKind, MismatchFix, Operand, Span};
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
        let (Ok(l), Ok(r)) = (l, r) else {
            return Err(Poisoned::of((!mismatched).then_some(Type::String)));
        };
        Ok(self.spilled(span, Type::String, vec![l, r], |operands| {
            let (lhs, rhs) = pair(operands);
            tir::ExprKind::Concat { lhs, rhs }
        }))
    }

    /// The operand `e` of the `+` over `operands`, of type `got`, which is not the `string` the
    /// other operand makes it. A value that interpolation prints gets the guess to interpolate
    /// it, unless the `+` is a `compound` assignment's or a constant's.
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
        if compound || self.in_constant || !printed {
            return self.mismatch(e.span, Some(e), &Type::String, got);
        }
        // shortcut: a parenthesized literal's span takes in its parentheses, which the edit
        // that drops its quote then cuts; upgrade once the AST keeps a literal's own span.
        let operand = |e: &ast::Expr| Operand {
            span: e.span,
            literal: matches!(e.kind, ExprKind::Str(_) | ExprKind::Interpolate(_)),
        };
        DiagnosticKind::Mismatch {
            want: vec![self.type_name(&Type::String)],
            got: self.type_name(got),
            at: e.span,
            fix: Some(MismatchFix::Interpolate {
                lhs: operand(lhs),
                rhs: operand(rhs),
            }),
        }
        .into()
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

    /// `receiver.method(args)` on a `string`, whose one method is `len()`.
    pub(super) fn string_method(
        &mut self,
        span: Span,
        receiver: Checked,
        method: &ast::Ident,
        args: &[ast::Arg],
    ) -> Checked {
        if method.name != "len" {
            let owner = self.type_name(&Type::String);
            let d = no_member(Definition::Method, owner, method, ["len"]);
            return self.rejected(d, args);
        }
        if !args.is_empty() {
            let d = DiagnosticKind::ArgumentCount {
                name: method.name.clone(),
                want: 0,
                found: args.len(),
                at: span,
            };
            return self.rejected(d.into(), args);
        }
        let kind = receiver.ok().map(|r| tir::ExprKind::Len(Box::new(r)));
        typed(span, Type::I64, false, kind)
    }

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
                    fix: None,
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
