//! Strings at run time. A string is a pointer to one block: a header of three `i64`s, the count,
//! the length and the capacity, then `cap` bytes. A literal's block is a writable global whose
//! count starts at 1, so it is never freed. This module is the only one that reads a string
//! header: it owns the layout, the string nodes and the bodies of the string helpers, which
//! `define_helpers` reaches through `define_string_helper`.

use inkwell::IntPredicate;
use inkwell::module::Linkage;
use inkwell::types::StructType;
use inkwell::values::{FunctionValue, IntValue, PointerValue};

use super::rc::{Helper, param};
use super::{Codegen, POSITIONED};
use crate::tir::{CompareOp, Expr, Type};

/// A string block's header, from the target's data layout. A block with capacity `cap` is
/// `size + cap` bytes, aligned to `align`.
struct StringLayout<'ctx> {
    /// `{ i64 count, i64 len, i64 cap, [0 x i8] bytes }`: the bytes start where the header ends.
    header: StructType<'ctx>,
    /// The header's ABI size.
    size: u64,
    /// The header's ABI alignment, which is the block's.
    align: u32,
}

/// The header's fields, by index.
const LEN: u32 = 1;
const CAP: u32 = 2;
const BYTES: u32 = 3;

impl<'ctx> Codegen<'ctx, '_> {
    fn string_layout(&self) -> StringLayout<'ctx> {
        let i64_type = self.context.i64_type().into();
        let bytes = self.context.i8_type().array_type(0).into();
        let header = self
            .context
            .struct_type(&[i64_type, i64_type, i64_type, bytes], false);
        StringLayout {
            header,
            size: self.target_data.get_abi_size(&header),
            align: self.target_data.get_abi_alignment(&header),
        }
    }

    /// The address of header field `index` of `string`.
    fn string_field(&self, string: PointerValue<'ctx>, index: u32) -> PointerValue<'ctx> {
        self.builder
            .build_struct_gep(self.string_layout().header, string, index, "field")
            .expect("the string header should have the length, capacity and bytes at 1, 2 and 3")
    }

    /// The `i64` header field `index` of `string`.
    fn load_string_field(&self, string: PointerValue<'ctx>, index: u32) -> IntValue<'ctx> {
        self.builder
            .build_load(
                self.context.i64_type(),
                self.string_field(string, index),
                "field",
            )
            .expect(POSITIONED)
            .into_int_value()
    }

    /// The length of `string` in bytes.
    pub(super) fn string_len(&self, string: PointerValue<'ctx>) -> IntValue<'ctx> {
        self.load_string_field(string, LEN)
    }

    /// `Str`: a private global `{ i64 1, i64 len, i64 len, [len x i8] }` for this literal node,
    /// retained. The global holds a reference of its own, so its count never reaches 0.
    pub(super) fn string_lit(&self, text: &str) -> PointerValue<'ctx> {
        let layout = self.string_layout();
        let i64_type = self.context.i64_type();
        let len = i64_type.const_int(
            u64::try_from(text.len()).expect("a literal should be shorter than 2^64 bytes"),
            false,
        );
        let block = self.context.const_struct(
            &[
                i64_type.const_int(1, false).into(),
                len.into(),
                len.into(),
                self.context.const_string(text.as_bytes(), false).into(),
            ],
            false,
        );
        let global = self.module.add_global(block.get_type(), None, "str");
        global.set_initializer(&block);
        // Retain and release write the count, so the block is not an LLVM `constant`.
        global.set_constant(false);
        global.set_alignment(layout.align);
        global.set_linkage(Linkage::Private);
        let string = global.as_pointer_value();
        self.retain(string.into(), &Type::String);
        string
    }

    /// `sisu_str_compare` on the bytes of `a` and `b`: -1, 0 or 1.
    fn str_compare(&self, a: PointerValue<'ctx>, b: PointerValue<'ctx>) -> IntValue<'ctx> {
        let args = [
            self.string_field(a, BYTES).into(),
            self.string_len(a).into(),
            self.string_field(b, BYTES).into(),
            self.string_len(b).into(),
        ];
        self.builder
            .build_call(self.str_compare, &args, "order")
            .expect(POSITIONED)
            .try_as_basic_value()
            .basic()
            .expect("`sisu_str_compare` should return an `i64`, as the runtime declares it")
            .into_int_value()
    }

    /// `lhs op rhs` on two strings: `sisu_str_compare`'s result against 0 under `op`'s signed
    /// predicate. Then both are released.
    pub(super) fn compare_strings(
        &mut self,
        op: CompareOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> IntValue<'ctx> {
        let (l, r) = (self.operand(lhs), self.operand(rhs));
        let order = self.str_compare(l.into_pointer_value(), r.into_pointer_value());
        let holds = self.compare_pair(op, true, order, self.context.i64_type().const_zero());
        self.release(l, &lhs.ty);
        self.release(r, &rhs.ty);
        holds
    }

    /// `print` of a string: `sisu_print_str(bytes, len)`, then the release.
    pub(super) fn print_string(&mut self, operand: &Expr) {
        let string = self.operand(operand).into_pointer_value();
        self.call(
            self.print_str,
            &[
                self.string_field(string, BYTES).into(),
                self.string_len(string).into(),
            ],
        );
        self.release(string.into(), &operand.ty);
    }

    /// The body of `@sisu_rc.drop.string` or `@sisu_rc.eq.string`.
    pub(super) fn define_string_helper(&self, kind: Helper, function: FunctionValue<'ctx>) {
        match kind {
            Helper::Drop => self.define_string_drop(function),
            Helper::Eq => self.define_string_eq(function),
            Helper::Release | Helper::Unique => {
                unreachable!("only a string's `drop` and `eq` should reach `define_string_helper`")
            }
        }
    }

    /// `@sisu_rc.drop.string`: frees the block, whose size is the header's plus the capacity.
    fn define_string_drop(&self, function: FunctionValue<'ctx>) {
        self.builder
            .position_at_end(self.context.append_basic_block(function, "entry"));
        let string = param(function);
        let layout = self.string_layout();
        let i64_type = self.context.i64_type();
        let size = self
            .builder
            .build_int_nuw_add(
                self.load_string_field(string, CAP),
                i64_type.const_int(layout.size, false),
                "size",
            )
            .expect(POSITIONED);
        self.call(
            self.free,
            &[
                string.into(),
                size.into(),
                i64_type.const_int(u64::from(layout.align), false).into(),
            ],
        );
        self.builder.build_return(None).expect(POSITIONED);
    }

    /// `@sisu_rc.eq.string(a, b)`: see `eq_entry` for the same pointer and `null`; otherwise
    /// the lengths, then the bytes by `sisu_str_compare`.
    fn define_string_eq(&self, function: FunctionValue<'ctx>) {
        let (a, b, equal, unequal) = self.eq_entry(function);
        let same_len = self.int_compare(IntPredicate::EQ, self.string_len(a), self.string_len(b));
        let bytes = self.context.append_basic_block(function, "bytes");
        self.builder
            .build_conditional_branch(same_len, bytes, unequal)
            .expect(POSITIONED);
        self.builder.position_at_end(bytes);
        let order = self.str_compare(a, b);
        let same = self.int_compare(
            IntPredicate::EQ,
            order,
            self.context.i64_type().const_zero(),
        );
        self.builder
            .build_conditional_branch(same, equal, unequal)
            .expect(POSITIONED);
    }
}
