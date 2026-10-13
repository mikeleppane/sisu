//! Checked integer operations: arithmetic that panics on overflow or a zero divisor, and
//! comparisons.

use inkwell::IntPredicate;
use inkwell::intrinsics::Intrinsic;
use inkwell::values::IntValue;

use super::{Codegen, POSITIONED};
use crate::diagnostic::Span;
use crate::tir::{BinaryOp, CompareOp, Expr, ExprKind, Type};

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
        let signed = lhs.ty != Type::U8;
        let (l, r) = (self.value(lhs), self.value(rhs));
        let intrinsic = |name| {
            format!(
                "llvm.{}{name}.with.overflow",
                if signed { 's' } else { 'u' }
            )
        };
        match op {
            BinaryOp::Add => self.checked(&intrinsic("add"), l, r, span),
            BinaryOp::Sub => self.checked(&intrinsic("sub"), l, r, span),
            BinaryOp::Mul => self.checked(&intrinsic("mul"), l, r, span),
            BinaryOp::Div | BinaryOp::Rem => self.division(op == BinaryOp::Rem, signed, l, r, span),
            BinaryOp::And | BinaryOp::Or => unreachable!(
                "`binary` should send `and` and `or` to `short_circuit` before this match"
            ),
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
            .expect(POSITIONED)
    }

    /// `lhs op rhs` through `intrinsic`, an `llvm.{s,u}*.with.overflow` overloaded on the
    /// operands' width; the overflow flag branches to an "integer overflow" panic.
    pub(super) fn checked(
        &self,
        intrinsic: &str,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        span: Span,
    ) -> IntValue<'ctx> {
        let (result, overflow) = self.overflowing(intrinsic, lhs, rhs);
        self.panic_if(overflow, span, "integer overflow");
        result
    }

    /// `lhs op rhs` through `intrinsic`, as for `checked`: the wrapped result and the
    /// overflow flag.
    pub(super) fn overflowing(
        &self,
        intrinsic: &str,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> (IntValue<'ctx>, IntValue<'ctx>) {
        let function = Intrinsic::find(intrinsic)
            .expect("LLVM should provide the `llvm.*.with.overflow` intrinsics by name")
            .get_declaration(&self.module, &[lhs.get_type().into()])
            .expect("the overflow intrinsics should be overloaded on any integer width");
        let pair = self
            .builder
            .build_call(function, &[lhs.into(), rhs.into()], "checked")
            .expect(POSITIONED)
            .try_as_basic_value()
            .basic()
            .expect("an overflow intrinsic should return a `{ value, overflow }` pair")
            .into_struct_value();
        let field = |index, name| {
            self.builder
                .build_extract_value(pair, index, name)
                .expect("the overflow pair should have a value at index 0 and a flag at index 1")
                .into_int_value()
        };
        (field(0, "result"), field(1, "overflow"))
    }

    /// `lhs / rhs` or `lhs % rhs`: panics on a zero divisor, then, when `signed`, on
    /// `i64::MIN / -1`, whose quotient does not fit (`sdiv` and `srem` are undefined for both).
    /// An unsigned division has no other failure.
    fn division(
        &self,
        rem: bool,
        signed: bool,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        span: Span,
    ) -> IntValue<'ctx> {
        let i64_type = self.context.i64_type();
        let zero = self.int_compare(IntPredicate::EQ, rhs, rhs.get_type().const_zero());
        self.panic_if(zero, span, "division by zero");
        if !signed {
            return if rem {
                self.builder.build_int_unsigned_rem(lhs, rhs, "rem")
            } else {
                self.builder.build_int_unsigned_div(lhs, rhs, "div")
            }
            .expect(POSITIONED);
        }
        let min = i64_type.const_int(i64::MIN.cast_unsigned(), false);
        let minus_one = i64_type.const_int((-1_i64).cast_unsigned(), false);
        let overflow = self
            .builder
            .build_and(
                self.int_compare(IntPredicate::EQ, lhs, min),
                self.int_compare(IntPredicate::EQ, rhs, minus_one),
                "overflow",
            )
            .expect(POSITIONED);
        self.panic_if(overflow, span, "integer overflow");
        if rem {
            self.builder.build_int_signed_rem(lhs, rhs, "rem")
        } else {
            self.builder.build_int_signed_div(lhs, rhs, "div")
        }
        .expect(POSITIONED)
    }

    pub(super) fn compare_pair(
        &self,
        op: CompareOp,
        signed: bool,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> IntValue<'ctx> {
        let predicate = match (op, signed) {
            (CompareOp::Lt, true) => IntPredicate::SLT,
            (CompareOp::Le, true) => IntPredicate::SLE,
            (CompareOp::Gt, true) => IntPredicate::SGT,
            (CompareOp::Ge, true) => IntPredicate::SGE,
            (CompareOp::Lt, false) => IntPredicate::ULT,
            (CompareOp::Le, false) => IntPredicate::ULE,
            (CompareOp::Gt, false) => IntPredicate::UGT,
            (CompareOp::Ge, false) => IntPredicate::UGE,
        };
        self.int_compare(predicate, lhs, rhs)
    }

    /// `i64(u8)` and `u8(i64)`, the target being `e.ty`; a `char` conversion lands in Task 9.
    /// Narrowing panics at `e.span` unless the value is in `0..=255`; one unsigned compare also
    /// catches the negatives.
    pub(super) fn convert(&mut self, e: &Expr) -> IntValue<'ctx> {
        let ExprKind::Convert(operand) = &e.kind else {
            unreachable!("`convert` should be called on a `Convert` node");
        };
        assert!(
            e.ty != Type::Char && operand.ty != Type::Char,
            "`char` lands in Task 9"
        );
        let value = self.value(operand);
        if e.ty == Type::I64 {
            return self
                .builder
                .build_int_z_extend(value, self.context.i64_type(), "widened")
                .expect(POSITIONED);
        }
        let too_big = self.int_compare(
            IntPredicate::UGT,
            value,
            value.get_type().const_int(u64::from(u8::MAX), false),
        );
        self.panic_if(too_big, e.span, "value does not fit in `u8`");
        self.builder
            .build_int_truncate(value, self.context.i8_type(), "narrowed")
            .expect(POSITIONED)
    }
}
