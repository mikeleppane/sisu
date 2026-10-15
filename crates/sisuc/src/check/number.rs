//! The methods on `i64` and `f64`, the constants on `f64`, and the warning for a comparison with
//! `f64.nan`.

use std::f64::consts;

use super::expr::{no_member, postfix, typed};
use super::{Checked, Checker, Poisoned, built_in_type};
use crate::ast;
use crate::diagnostic::{Definition, DiagnosticKind, NanFix, Span};
use crate::tir::{self, MathOp, Type};

/// The receivers of a method.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Receivers {
    F64,
    /// `f64` and `i64`.
    Number,
}

/// Each method: its operation, its receivers and its parameter count. Every parameter has the
/// receiver's type. The result is `bool` for the `is_` methods, the receiver's type for `min`,
/// `max` and `clamp`, and `f64` for the rest.
const METHODS: [(MathOp, Receivers, usize); 25] = [
    (MathOp::Abs, Receivers::F64, 0),
    (MathOp::Sqrt, Receivers::F64, 0),
    (MathOp::Floor, Receivers::F64, 0),
    (MathOp::Ceil, Receivers::F64, 0),
    (MathOp::Trunc, Receivers::F64, 0),
    (MathOp::Round, Receivers::F64, 0),
    (MathOp::Sin, Receivers::F64, 0),
    (MathOp::Cos, Receivers::F64, 0),
    (MathOp::Tan, Receivers::F64, 0),
    (MathOp::Asin, Receivers::F64, 0),
    (MathOp::Acos, Receivers::F64, 0),
    (MathOp::Atan, Receivers::F64, 0),
    (MathOp::Atan2, Receivers::F64, 1),
    (MathOp::Exp, Receivers::F64, 0),
    (MathOp::Ln, Receivers::F64, 0),
    (MathOp::Log10, Receivers::F64, 0),
    (MathOp::Log2, Receivers::F64, 0),
    (MathOp::Pow, Receivers::F64, 1),
    (MathOp::Hypot, Receivers::F64, 1),
    (MathOp::Min, Receivers::Number, 1),
    (MathOp::Max, Receivers::Number, 1),
    (MathOp::Clamp, Receivers::Number, 2),
    (MathOp::IsNan, Receivers::F64, 0),
    (MathOp::IsFinite, Receivers::F64, 0),
    (MathOp::IsInfinite, Receivers::F64, 0),
];

/// The constants on `f64`: `f64.pi` is `PI`.
const CONSTANTS: [(&str, f64); 8] = [
    ("pi", consts::PI),
    ("tau", consts::TAU),
    ("e", consts::E),
    ("infinity", f64::INFINITY),
    ("nan", f64::NAN),
    ("max", f64::MAX),
    ("min_positive", f64::MIN_POSITIVE),
    ("epsilon", f64::EPSILON),
];

impl Checker<'_> {
    /// `base.member` where `base` names a built-in type that no local shadows: the `f64`
    /// constant `member`, or, for any other member, an error whose candidates are that type's
    /// constants (only `f64` has any). `None` when `base` is not a type name.
    pub(super) fn type_path(
        &mut self,
        span: Span,
        base: &ast::Expr,
        member: &ast::Ident,
    ) -> Option<Checked> {
        let ast::ExprKind::Name(name) = base.kind else {
            return None;
        };
        let ty = built_in_type(name.as_str())?;
        if self.lookup(name).is_some() {
            return None;
        }
        let constants: &[(&str, f64)] = if ty == Type::F64 { &CONSTANTS } else { &[] };
        let Some(&(_, x)) = constants
            .iter()
            .find(|(constant, _)| *constant == member.name.as_str())
        else {
            let names = constants.iter().map(|(constant, _)| *constant);
            let d = no_member(Definition::Constant, ty.to_string(), member, names);
            self.diagnostics.push(d);
            return Some(Err(Poisoned::error()));
        };
        Some(Ok(tir::Expr {
            kind: tir::ExprKind::Float(x),
            ty: Type::F64,
            span,
        }))
    }

    /// Warns about `lhs == rhs`, or `!=` when `negated`, when a side is `f64.nan`: no number
    /// equals it, so the answer is fixed. Each side is its source and its checked form. An
    /// `f64` on the other side is tested with `is_nan()`; an `f64?` is not, as the call would
    /// not check it. A side that failed, or has another type, is reported elsewhere.
    pub(super) fn nan_comparison(
        &mut self,
        negated: bool,
        lhs: (&ast::Expr, &Checked),
        rhs: (&ast::Expr, &Checked),
    ) {
        // `f64.nan` is the only member that checks to a `nan`.
        let is_nan = |(e, checked): (&ast::Expr, &Checked)| {
            matches!(e.kind, ast::ExprKind::Field { .. })
                && matches!(checked, Ok(tir::Expr { kind: tir::ExprKind::Float(x), .. }) if x.is_nan())
        };
        let other = if is_nan(lhs) {
            rhs
        } else if is_nan(rhs) {
            lhs
        } else {
            return;
        };
        let fix = match other.1.as_ref().ok().map(|checked| &checked.ty) {
            Some(Type::F64) => Some(NanFix {
                other: other.0.span,
                wrap: !postfix(other.0),
            }),
            Some(Type::Optional(payload)) if **payload == Type::F64 => None,
            _ => return,
        };
        let at = lhs.0.span.to(rhs.0.span);
        self.diagnostics
            .push(DiagnosticKind::NanComparison { at, negated, fix }.into());
    }

    /// `receiver.method(args)` on an `i64` or an `f64`, `ty`. Each argument is checked with
    /// `ty` as its expected type, so `x.pow(2)` makes the `2` an `f64`.
    pub(super) fn number_method(
        &mut self,
        span: Span,
        receiver: Checked,
        ty: &Type,
        method: &ast::Ident,
        args: &[ast::Arg],
    ) -> Checked {
        let float = *ty == Type::F64;
        let methods = || {
            METHODS
                .iter()
                .filter(move |(_, receivers, _)| float || *receivers == Receivers::Number)
        };
        let Some(&(op, _, params)) = methods().find(|(op, ..)| op.name() == method.name.as_str())
        else {
            let owner = self.type_name(ty);
            let names = methods().map(|(op, ..)| op.name());
            return self.rejected(no_member(Definition::Method, owner, method, names), args);
        };
        if args.len() != params {
            let d = DiagnosticKind::ArgumentCount {
                name: method.name.to_string(),
                want: params,
                found: args.len(),
                at: span,
            };
            return self.rejected(d.into(), args);
        }
        let labeled = self.labels(args);
        let (checked, mismatched): (Vec<_>, Vec<_>) = args
            .iter()
            .map(|arg| self.operand(&arg.value, Some(ty)))
            .unzip();
        let result = match op {
            MathOp::IsNan | MathOp::IsFinite | MathOp::IsInfinite => Type::Bool,
            MathOp::Min | MathOp::Max | MathOp::Clamp => ty.clone(),
            _ => Type::F64,
        };
        let failed = labeled || mismatched.contains(&true);
        let operands: Result<Vec<_>, Poisoned> = std::iter::once(receiver).chain(checked).collect();
        // Never an owned reference, so no operand needs `spill`.
        let kind = operands
            .ok()
            .filter(|_| !failed)
            .map(|operands| tir::ExprKind::Math { op, operands });
        typed(span, result, failed, kind)
    }
}
