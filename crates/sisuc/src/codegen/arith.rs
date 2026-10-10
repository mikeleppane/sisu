//! Checked integer operations: arithmetic that panics on overflow or a zero divisor, and
//! comparisons.

use inkwell::IntPredicate;
use inkwell::intrinsics::Intrinsic;
use inkwell::values::IntValue;

use super::Codegen;
use crate::diagnostic::Span;
use crate::tir::{BinaryOp, CompareOp, Expr};

impl<'ctx> Codegen<'ctx, '_> {
    /// `span` is the operator expression's: arithmetic panics point at its start.
    pub(super) fn binary(
        &mut self,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
    ) -> IntValue<'ctx> {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            return self.short_circuit(op == BinaryOp::And, lhs, rhs);
        }
        let (l, r) = (self.value(lhs), self.value(rhs));
        match op {
            BinaryOp::Add => self.checked("llvm.sadd.with.overflow", l, r, span),
            BinaryOp::Sub => self.checked("llvm.ssub.with.overflow", l, r, span),
            BinaryOp::Mul => self.checked("llvm.smul.with.overflow", l, r, span),
            BinaryOp::Div | BinaryOp::Rem => self.division(op == BinaryOp::Rem, l, r, span),
            BinaryOp::And | BinaryOp::Or => unreachable!("handled above"),
        }
    }

    pub(super) fn int_compare(
        &self,
        predicate: IntPredicate,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> IntValue<'ctx> {
        self.builder
            .build_int_compare(predicate, lhs, rhs, "cmp")
            .expect("builder is positioned")
    }

    /// `lhs op rhs` through `intrinsic`, an `llvm.s*.with.overflow` that returns the wrapped
    /// result and an overflow flag; the flag branches to an "integer overflow" panic.
    pub(super) fn checked(
        &self,
        intrinsic: &str,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        span: Span,
    ) -> IntValue<'ctx> {
        let function = Intrinsic::find(intrinsic)
            .expect("LLVM has the overflow intrinsics")
            .get_declaration(&self.module, &[self.context.i64_type().into()])
            .expect("the intrinsic is overloaded on i64");
        let pair = self
            .builder
            .build_call(function, &[lhs.into(), rhs.into()], "checked")
            .expect("builder is positioned")
            .try_as_basic_value()
            .basic()
            .expect("the intrinsic returns a pair")
            .into_struct_value();
        let field = |index, name| {
            self.builder
                .build_extract_value(pair, index, name)
                .expect("the pair has two fields")
                .into_int_value()
        };
        let (result, overflow) = (field(0, "result"), field(1, "overflow"));
        self.panic_if(overflow, span, "integer overflow");
        result
    }

    /// `lhs / rhs` or `lhs % rhs`: panics on a zero divisor, then on `i64::MIN / -1`, whose
    /// quotient does not fit (`sdiv` and `srem` are undefined for both).
    fn division(
        &self,
        rem: bool,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        span: Span,
    ) -> IntValue<'ctx> {
        let i64_type = self.context.i64_type();
        let zero = self.int_compare(IntPredicate::EQ, rhs, i64_type.const_zero());
        self.panic_if(zero, span, "division by zero");
        let min = i64_type.const_int(i64::MIN.cast_unsigned(), false);
        let minus_one = i64_type.const_int((-1_i64).cast_unsigned(), false);
        let overflow = self
            .builder
            .build_and(
                self.int_compare(IntPredicate::EQ, lhs, min),
                self.int_compare(IntPredicate::EQ, rhs, minus_one),
                "overflow",
            )
            .expect("builder is positioned");
        self.panic_if(overflow, span, "integer overflow");
        if rem {
            self.builder.build_int_signed_rem(lhs, rhs, "rem")
        } else {
            self.builder.build_int_signed_div(lhs, rhs, "div")
        }
        .expect("builder is positioned")
    }

    pub(super) fn compare_pair(
        &self,
        op: CompareOp,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> IntValue<'ctx> {
        let predicate = match op {
            CompareOp::Lt => IntPredicate::SLT,
            CompareOp::Le => IntPredicate::SLE,
            CompareOp::Gt => IntPredicate::SGT,
            CompareOp::Ge => IntPredicate::SGE,
        };
        self.builder
            .build_int_compare(predicate, lhs, rhs, "cmp")
            .expect("builder is positioned")
    }
}
