//! Top-level constants: the gate on what a value may hold, and folding it as run time would.
//! Each use of a constant lowers to its literal, so `tir` and codegen never see a constant.

use super::{Checked, Checker, Declared, Poisoned, type_of};
use crate::ast::{self, ExprKind};
use crate::diagnostic::{Diagnostic, DiagnosticKind, Panic, Span};
use crate::tir::{self, BinaryOp, CompareOp, Type, UnaryOp};

/// A folded constant's value; the variant fixes the type.
#[derive(Clone, Copy, PartialEq)]
enum Constant {
    Int(i64),
    U8(u8),
    Bool(bool),
}

impl Constant {
    /// The integer `n` as a value of the integer type `ty`, or `None` when it does not fit.
    fn of(n: i64, ty: &Type) -> Option<Constant> {
        if *ty == Type::U8 {
            u8::try_from(n).ok().map(Constant::U8)
        } else {
            Some(Constant::Int(n))
        }
    }

    fn ty(self) -> Type {
        match self {
            Constant::Int(_) => Type::I64,
            Constant::U8(_) => Type::U8,
            Constant::Bool(_) => Type::Bool,
        }
    }

    /// The literal node that a use at `span` lowers to.
    fn lower(self, span: Span) -> tir::Expr {
        let kind = match self {
            Constant::Int(n) => tir::ExprKind::Int(n),
            Constant::U8(n) => tir::ExprKind::Int(i64::from(n)),
            Constant::Bool(b) => tir::ExprKind::Bool(b),
        };
        tir::Expr {
            kind,
            ty: self.ty(),
            span,
        }
    }

    /// The value of an `i64` or a `u8`.
    fn int(self) -> i64 {
        match self {
            Constant::Int(n) => n,
            Constant::U8(n) => i64::from(n),
            Constant::Bool(_) => {
                unreachable!("the checker should give an integer operator integers")
            }
        }
    }

    fn bool(self) -> bool {
        match self {
            Constant::Bool(b) => b,
            Constant::Int(_) | Constant::U8(_) => {
                unreachable!("the checker should give a `bool` operator `bool`s")
            }
        }
    }
}

pub(super) struct ConstInfo {
    /// The name's span, for "first defined here" and "declared with `const` here".
    pub(super) span: Span,
    /// Source order among the constants.
    order: usize,
    /// The `: T` if given, else the folded value's type; `None` while unknown.
    pub(super) ty: Option<Type>,
    /// `None` until folded, and for good once folding fails.
    value: Option<Constant>,
}

impl ConstInfo {
    /// The constant named at `span`, at `order` among the constants, not yet folded.
    pub(super) fn new(span: Span, order: usize) -> Self {
        Self {
            span,
            order,
            ty: None,
            value: None,
        }
    }
}

impl Checker {
    /// Folds every constant in source order.
    pub(super) fn fold_constants(&mut self, program: &ast::Program) {
        let constants = program.items.iter().filter_map(|item| match item {
            ast::Item::Const(c) => Some(c),
            ast::Item::Function(_) | ast::Item::Class(_) => None,
        });
        for (order, c) in constants.enumerate() {
            let (ty, value) = self.fold(c, order);
            // A duplicate is folded for its own errors only.
            if let Some(Declared::Known(info)) = self.constants.get_mut(&c.name.name)
                && info.span == c.name.span
            {
                info.ty = ty;
                info.value = value;
            }
        }
    }

    /// The type and value of `c`, the constant at `order`; each is `None` when not known. The
    /// first step that fails reports its errors, and the rest do not run.
    fn fold(&mut self, c: &ast::Const, order: usize) -> (Option<Type>, Option<Constant>) {
        let declared = match &c.ty {
            None => None,
            Some(ty) => match self.resolve(ty) {
                None => return (None, None),
                Some(resolved @ (Type::I64 | Type::U8 | Type::Bool)) => Some(resolved),
                Some(_) => {
                    self.report(DiagnosticKind::ConstantType { at: ty.span });
                    return (None, None);
                }
            },
        };
        if !self.gate(&c.value, &c.name, order) {
            return (declared, None);
        }
        let checked = match &declared {
            Some(ty) => self.expect(&c.value, Some(ty)),
            None => self.value(&c.value, None),
        };
        let ty = declared.or_else(|| type_of(&checked).cloned());
        let Ok(checked) = checked else {
            return (ty, None);
        };
        match evaluate(&checked) {
            Ok(value) => (ty, Some(value)),
            Err(d) => {
                self.diagnostics.push(d);
                (ty, None)
            }
        }
    }

    /// Whether `e`, in the value of the constant `user` at `order`, holds only literals,
    /// operators, conversions and names. Reports every part that does not, without entering it,
    /// and every use of `user` itself or of a constant below it.
    fn gate(&mut self, e: &ast::Expr, user: &ast::Ident, order: usize) -> bool {
        // Every part is gated, so that each one that is not known is reported.
        let all = |c: &mut Self, parts: &mut dyn Iterator<Item = &ast::Expr>| {
            parts.filter(|part| !c.gate(part, user, order)).count() == 0
        };
        match &e.kind {
            ExprKind::Int { .. } | ExprKind::Bool(_) => true,
            ExprKind::Unary { operand, .. } => self.gate(operand, user, order),
            ExprKind::Binary { lhs, rhs, .. } => all(self, &mut [&**lhs, &**rhs].into_iter()),
            ExprKind::Compare { operands, .. } => all(self, &mut operands.iter()),
            ExprKind::Call { callee, args } if matches!(callee.name.as_str(), "i64" | "u8") => {
                all(self, &mut args.iter().map(|arg| &arg.value))
            }
            ExprKind::Name(name) => {
                let declared = if *name == user.name {
                    user.span
                } else {
                    match self.constants.get(name) {
                        Some(Declared::Known(info)) if info.order > order => info.span,
                        _ => return true,
                    }
                };
                self.report(DiagnosticKind::ConstantOrder {
                    name: name.clone(),
                    user: user.name.clone(),
                    at: e.span,
                    declared,
                    itself: *name == user.name,
                });
                false
            }
            _ => {
                self.report(DiagnosticKind::ConstantNotKnown { at: e.span });
                false
            }
        }
    }

    /// A use of `name` at `span` if it names a constant: its literal, or, if folding failed,
    /// `Poisoned` of its `ty`, with no diagnostic.
    pub(super) fn constant_use(&self, name: &str, span: Span) -> Option<Checked> {
        Some(match self.constants.get(name)? {
            Declared::Known(ConstInfo {
                value: Some(value), ..
            }) => Ok(value.lower(span)),
            Declared::Known(info) => Err(Poisoned::of(info.ty.clone())),
            Declared::Broken => Err(Poisoned::error()),
        })
    }
}

/// Runs a checked initializer as run time would: operands in order, `&&` and `||` and a
/// comparison chain stopping where run time stops, and a panic as the error at its node.
fn evaluate(e: &tir::Expr) -> Result<Constant, Diagnostic> {
    let panic =
        |panic| -> Diagnostic { DiagnosticKind::ConstantPanics { panic, at: e.span }.into() };
    // An integer result in the node's type; one that does not fit overflows.
    let int = |n: Option<i64>| {
        n.and_then(|n| Constant::of(n, &e.ty))
            .ok_or_else(|| panic(Panic::Overflow))
    };
    Ok(match &e.kind {
        tir::ExprKind::Int(n) => {
            Constant::of(*n, &e.ty).expect("the checker should fit a literal in its type")
        }
        tir::ExprKind::Bool(b) => Constant::Bool(*b),
        tir::ExprKind::Unary { op, operand } => {
            let operand = evaluate(operand)?;
            match op {
                UnaryOp::Not => Constant::Bool(!operand.bool()),
                UnaryOp::Neg => int(operand.int().checked_neg())?,
            }
        }
        tir::ExprKind::Binary {
            op: BinaryOp::And,
            lhs,
            rhs,
        } => Constant::Bool(evaluate(lhs)?.bool() && evaluate(rhs)?.bool()),
        tir::ExprKind::Binary {
            op: BinaryOp::Or,
            lhs,
            rhs,
        } => Constant::Bool(evaluate(lhs)?.bool() || evaluate(rhs)?.bool()),
        tir::ExprKind::Binary { op, lhs, rhs } => {
            let (l, r) = (evaluate(lhs)?.int(), evaluate(rhs)?.int());
            if matches!(op, BinaryOp::Div | BinaryOp::Rem) && r == 0 {
                return Err(panic(Panic::DivisionByZero));
            }
            // Each operand fits in an `i64`, so a `u8` result overflows only out of `u8`.
            int(match op {
                BinaryOp::Add => l.checked_add(r),
                BinaryOp::Sub => l.checked_sub(r),
                BinaryOp::Mul => l.checked_mul(r),
                BinaryOp::Div => l.checked_div(r),
                BinaryOp::Rem => l.checked_rem(r),
                BinaryOp::And | BinaryOp::Or => {
                    unreachable!("`&&` and `||` should take the arms above")
                }
            })?
        }
        tir::ExprKind::Compare { operands, ops } => {
            let mut lhs = evaluate(&operands[0])?.int();
            for (op, operand) in ops.iter().zip(&operands[1..]) {
                let rhs = evaluate(operand)?.int();
                let holds = match op {
                    CompareOp::Lt => lhs < rhs,
                    CompareOp::Le => lhs <= rhs,
                    CompareOp::Gt => lhs > rhs,
                    CompareOp::Ge => lhs >= rhs,
                };
                if !holds {
                    return Ok(Constant::Bool(false));
                }
                lhs = rhs;
            }
            Constant::Bool(true)
        }
        tir::ExprKind::Equal { negated, lhs, rhs } => {
            Constant::Bool((evaluate(lhs)? == evaluate(rhs)?) != *negated)
        }
        tir::ExprKind::Convert(operand) => {
            Constant::of(evaluate(operand)?.int(), &e.ty).ok_or_else(|| panic(Panic::NotU8))?
        }
        _ => unreachable!("the gate should let only literals, operators and conversions through"),
    })
}
