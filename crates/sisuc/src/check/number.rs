//! The methods on `i64` and `f64`.

use super::expr::{no_member, typed};
use super::{Checked, Checker, Poisoned};
use crate::ast;
use crate::diagnostic::{Definition, DiagnosticKind, Span};
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

impl Checker<'_> {
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
