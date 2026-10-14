//! Strings at run time. A string is a pointer to one block: a header of three `i64`s, the count,
//! the length and the capacity, then `cap` bytes. A literal's block is a writable global that
//! holds a reference of its own, so it is never freed and never reused in place. This module is the only one that reads a string
//! header: it owns the layout, the string nodes and the bodies of the string helpers, which
//! `define_helpers` reaches through `define_string_helper`.

use inkwell::attributes::AttributeLoc;
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, StructType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue,
};
use inkwell::{AddressSpace, IntPredicate};

use super::rc::{Helper, param};
use super::{Codegen, POSITIONED, add_nounwind_memory, zeroext};
use crate::diagnostic::Span;
use crate::tir::{CompareOp, Expr, Place, Stream, Type};

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

    /// The address of byte `offset` of `string`, which is at most its capacity.
    #[expect(
        unsafe_code,
        reason = "inkwell marks GEP unsafe for a wrong index list; this GEP takes one `i64` index"
    )]
    fn string_byte(
        &self,
        string: PointerValue<'ctx>,
        offset: IntValue<'ctx>,
    ) -> PointerValue<'ctx> {
        // SAFETY: the one `i64` index steps over `i8`s, so the index list is valid for its
        // pointee type. `inbounds` is sound because the block holds `cap` bytes after the
        // header and every caller passes an offset of at most `cap`, one past the end at most.
        unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    self.string_field(string, BYTES),
                    &[offset],
                    "byte",
                )
                .expect(POSITIONED)
        }
    }

    /// `Concat`: evaluates `lhs`, then `rhs`, and joins them with `concat`.
    pub(super) fn concat_expr(&mut self, lhs: &Expr, rhs: &Expr) -> PointerValue<'ctx> {
        let (l, r) = (self.operand(lhs), self.operand(rhs));
        self.concat(l.into_pointer_value(), r.into_pointer_value())
    }

    /// `Append { place, value }`, in the order that gives `P + value` its left-to-right meaning:
    /// the place's operands; a bounds-checked read of `P`, retained; `value`; the walk that
    /// makes the path unique; the release of the place's own reference, so that `concat` checks
    /// the count last; the store of the result; and the release of a field root's base.
    pub(super) fn append(&mut self, place: &Place, value: &Expr) {
        let operands = self.place_operands(place).expect(
            "the checker should only make an append whose indexes are locals or literals, which cannot exit",
        );
        let ptr_type = self.context.ptr_type(AddressSpace::default()).into();
        let slot = self.read_slot(operands.slot, &operands.ty, &operands.indexes);
        let read = self.load(ptr_type, slot);
        self.retain(read, &Type::String);
        let text = self
            .expr(value)
            .expect("the checker should only make an append whose value cannot exit");
        let (slot, _) = self.element_slot(operands.slot, &operands.ty, &operands.indexes);
        self.release(self.load(ptr_type, slot), &Type::String);
        let result = self.concat(read.into_pointer_value(), text.into_pointer_value());
        self.store(slot, result.into());
        operands.release_object(self);
    }

    /// `sisu_realloc(ptr, old_size, align, new_size) -> ptr` from the runtime, declared on
    /// first use, so that a module without `+` stays as it was.
    fn realloc(&self) -> FunctionValue<'ctx> {
        self.module.get_function("sisu_realloc").unwrap_or_else(|| {
            let ptr_type = self.context.ptr_type(AddressSpace::default());
            let i64_type = self.context.i64_type().into();
            let params = [ptr_type.into(), i64_type, i64_type, i64_type];
            let fn_type = ptr_type.fn_type(&params, false);
            self.module.add_function("sisu_realloc", fn_type, None)
        })
    }

    /// `lhs + rhs`, which takes both owned references and returns an owned one. When
    /// the count of `lhs` is 1, the bytes of `rhs` are appended into its block, which is first
    /// reallocated to `max(needed, 2 * cap)` when its capacity is below the `needed` length.
    /// Otherwise a new block with capacity `needed` receives both byte runs, and `lhs` is
    /// released. Either way `rhs` is released last.
    pub(super) fn concat(
        &self,
        lhs: PointerValue<'ctx>,
        rhs: PointerValue<'ctx>,
    ) -> PointerValue<'ctx> {
        let layout = self.string_layout();
        let i64_type = self.context.i64_type();
        let header = i64_type.const_int(layout.size, false);
        let align = i64_type.const_int(u64::from(layout.align), false);
        let block_size = |cap| {
            self.builder
                .build_int_nuw_add(header, cap, "size")
                .expect(POSITIONED)
        };
        let (len, rhs_len) = (self.string_len(lhs), self.string_len(rhs));
        // Both strings exist in memory, which on x86-64 is below 2^47 bytes, so no length,
        // capacity or block size here can overflow.
        let needed = self
            .builder
            .build_int_nuw_add(len, rhs_len, "needed")
            .expect(POSITIONED);
        let function = self.current_function();
        let block = |name| self.context.append_basic_block(function, name);
        let (owned, grow, copy, join) =
            (block("owned"), block("grow"), block("copy"), block("join"));
        let count = self
            .builder
            .build_load(i64_type, lhs, "count")
            .expect(POSITIONED)
            .into_int_value();
        let one = self.int_compare(IntPredicate::EQ, count, i64_type.const_int(1, false));
        self.builder
            .build_conditional_branch(one, owned, copy)
            .expect(POSITIONED);

        // Both operands are owned references, so with a count of 1 `lhs` is not `rhs`'s block:
        // reallocating it leaves `rhs` in place.
        self.builder.position_at_end(owned);
        let cap = self.load_string_field(lhs, CAP);
        let full = self.int_compare(IntPredicate::ULT, cap, needed);
        self.builder
            .build_conditional_branch(full, grow, join)
            .expect(POSITIONED);

        self.builder.position_at_end(grow);
        let double = self
            .builder
            .build_int_nuw_mul(cap, i64_type.const_int(2, false), "double")
            .expect(POSITIONED);
        let more = self.int_compare(IntPredicate::UGT, needed, double);
        let new_cap = self
            .builder
            .build_select(more, needed, double, "cap")
            .expect(POSITIONED)
            .into_int_value();
        let grown = self
            .builder
            .build_call(
                self.realloc(),
                &[
                    lhs.into(),
                    block_size(cap).into(),
                    align.into(),
                    block_size(new_cap).into(),
                ],
                "grown",
            )
            .expect(POSITIONED)
            .try_as_basic_value()
            .basic()
            .expect("`sisu_realloc` should return a pointer, as the runtime declares it")
            .into_pointer_value();
        self.store(self.string_field(grown, CAP), new_cap.into());
        self.branch_to(join);

        self.builder.position_at_end(copy);
        let fresh = self.new_string(needed);
        self.copy_bytes(self.string_field(fresh, BYTES), lhs, len);
        self.release(lhs.into(), &Type::String);
        let copied = self.current_block();
        self.branch_to(join);

        self.builder.position_at_end(join);
        let result = self
            .builder
            .build_phi(self.context.ptr_type(AddressSpace::default()), "string")
            .expect(POSITIONED);
        result.add_incoming(&[(&lhs, owned), (&grown, grow), (&fresh, copied)]);
        let result = result.as_basic_value().into_pointer_value();
        self.copy_bytes(self.string_byte(result, len), rhs, rhs_len);
        self.store(self.string_field(result, LEN), needed.into());
        self.release(rhs.into(), &Type::String);
        result
    }

    /// A new string holding a copy of the `len` bytes at `bytes`, which are valid UTF-8.
    pub(super) fn string_from_bytes(
        &self,
        bytes: PointerValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> PointerValue<'ctx> {
        let string = self.new_string(len);
        self.builder
            .build_memcpy(self.string_field(string, BYTES), 1, bytes, 1, len)
            .expect("an alignment of 1 should be a power of two");
        self.store(self.string_field(string, LEN), len.into());
        string
    }

    /// A new block with count 1 and capacity `cap`, whose length is not yet stored.
    fn new_string(&self, cap: IntValue<'ctx>) -> PointerValue<'ctx> {
        let layout = self.string_layout();
        let i64_type = self.context.i64_type();
        let size = self
            .builder
            .build_int_nuw_add(i64_type.const_int(layout.size, false), cap, "size")
            .expect(POSITIONED);
        let align = i64_type.const_int(u64::from(layout.align), false);
        let string = self
            .builder
            .build_call(self.alloc, &[size.into(), align.into()], "string")
            .expect(POSITIONED)
            .try_as_basic_value()
            .basic()
            .expect("`sisu_alloc` should return a pointer, as the runtime declares it")
            .into_pointer_value();
        self.store(string, i64_type.const_int(1, false).into());
        self.store(self.string_field(string, CAP), cap.into());
        string
    }

    /// `Interpolate`, in one pass: evaluates every part, left to right, then allocates one
    /// block with room for the most text each part can give, writes each part at the running
    /// offset, stores the length written and releases the `string` parts. `None` when a part
    /// exits: nothing is allocated yet, and the checker bound every counted part before it to
    /// a local, which the exit releases.
    pub(super) fn interpolate(&mut self, parts: &[Expr]) -> Option<BasicValueEnum<'ctx>> {
        let mut values = Vec::with_capacity(parts.len());
        for part in parts {
            values.push(self.expr(part)?);
        }
        let i64_type = self.context.i64_type();
        let add = |a, b| {
            self.builder
                .build_int_nuw_add(a, b, "len")
                .expect(POSITIONED)
        };
        // The most bytes each part's text takes: see `sisu_write_int`, `_bool` and `_char`. A
        // string's length is below 2^47 on x86-64, so the sum cannot overflow.
        let cap = parts
            .iter()
            .zip(&values)
            .fold(i64_type.const_zero(), |cap, (part, value)| {
                let bound = match part.ty {
                    Type::String => return add(cap, self.string_len(value.into_pointer_value())),
                    Type::I64 => 20,
                    Type::U8 => 3,
                    Type::Bool => 5,
                    Type::Char => 4,
                    _ => unreachable!(
                        "the checker should accept only text, integers, `bool` and `char`"
                    ),
                };
                add(cap, i64_type.const_int(bound, false))
            });
        let result = self.new_string(cap);
        let mut len = i64_type.const_zero();
        for (part, value) in parts.iter().zip(&values) {
            let at = self.string_byte(result, len);
            let written = match part.ty {
                Type::String => {
                    let string = value.into_pointer_value();
                    let part_len = self.string_len(string);
                    self.copy_bytes(at, string, part_len);
                    part_len
                }
                Type::U8 => {
                    let widened = self
                        .builder
                        .build_int_z_extend(value.into_int_value(), i64_type, "widened")
                        .expect(POSITIONED);
                    self.write_text("sisu_write_int", widened.into(), at)
                }
                Type::I64 => self.write_text("sisu_write_int", *value, at),
                Type::Bool => self.write_text("sisu_write_bool", *value, at),
                _ => self.write_text("sisu_write_char", *value, at),
            };
            len = add(len, written);
        }
        self.store(self.string_field(result, LEN), len.into());
        for (part, value) in parts.iter().zip(values) {
            self.discard(Some(value), &part.ty);
        }
        Some(result.into())
    }

    /// `base`, a string or an array, borrowed when it is a local, and whether it was: the
    /// caller releases it otherwise. For a read that ends before anything can replace the base.
    fn borrow_pointer(&mut self, base: &Expr) -> (PointerValue<'ctx>, bool) {
        let (value, borrowed) = self.borrow_or_expr(base);
        let value = value.expect("the checker should only give these nodes a string or an array, which lower to a pointer");
        (value.into_pointer_value(), borrowed)
    }

    /// `Chars`: sizes the `[char]` by `sisu_utf8_count`, allocates it, and has
    /// `sisu_utf8_decode` fill it. The string is released after.
    pub(super) fn chars(&mut self, base: &Expr) -> PointerValue<'ctx> {
        let (string, borrowed) = self.borrow_pointer(base);
        let (bytes, len) = (self.string_field(string, BYTES), self.string_len(string));
        let count = self.utf8_size("sisu_utf8_count", bytes, len);
        let array = self.alloc_array(&Type::Char, count, None);
        let first = self.element_ptr(&Type::Char, array, self.context.i64_type().const_zero());
        self.utf8_fill("sisu_utf8_decode", bytes, len, first);
        if !borrowed {
            self.release(string.into(), &base.ty);
        }
        array
    }

    /// `Bytes`: a `[u8]` as long as the string, with its bytes copied in. The string is
    /// released after.
    pub(super) fn bytes(&mut self, base: &Expr) -> PointerValue<'ctx> {
        let (string, borrowed) = self.borrow_pointer(base);
        let len = self.string_len(string);
        let array = self.alloc_array(&Type::U8, len, None);
        let first = self.element_ptr(&Type::U8, array, self.context.i64_type().const_zero());
        self.copy_bytes(first, string, len);
        if !borrowed {
            self.release(string.into(), &base.ty);
        }
        array
    }

    /// `Words`: counts the words with `sisu_words_count`, which also checks the whole string,
    /// allocates the `[string]`, and fills each element from a `sisu_word_next` call that starts
    /// where the last word ended, so the work is linear. The string is released after.
    pub(super) fn words(&mut self, base: &Expr) -> PointerValue<'ctx> {
        let (string, borrowed) = self.borrow_pointer(base);
        let i64_type = self.context.i64_type();
        let (bytes, len) = (self.string_field(string, BYTES), self.string_len(string));
        let count = self.call_word_function("sisu_words_count", &[bytes.into(), len.into()], 0b01);
        let array = self.alloc_array(&Type::String, count, None);
        // `count_loop` gives its body only the index, so the offset where the last word ended,
        // and the start of the word just found, live in slots.
        let from = self.entry_alloca(i64_type.into(), "from");
        let start = self.entry_alloca(i64_type.into(), "start");
        self.store(from, i64_type.const_zero().into());
        self.count_loop(count, |index| {
            let args = [
                bytes.into(),
                len.into(),
                self.load(i64_type.into(), from).into(),
                start.into(),
            ];
            let end = self.call_word_function("sisu_word_next", &args, 0b11);
            let first = self.load(i64_type.into(), start).into_int_value();
            let word_len = self
                .builder
                .build_int_nuw_sub(end, first, "word_len")
                .expect(POSITIONED);
            let word = self.string_from_bytes(self.string_byte(string, first), word_len);
            let slot = self.element_ptr(&Type::String, array, index);
            self.store(slot, word.into());
            self.store(from, end.into());
        });
        if !borrowed {
            self.release(string.into(), &base.ty);
        }
        array
    }

    /// A call of the runtime's `name`, which returns an `i64`: `sisu_words_count(bytes, len)`
    /// or `sisu_word_next(bytes, len, from, start)`. Declared on first use, `nounwind`, with the
    /// `memory` attribute value `memory`: the first only reads the memory behind its arguments,
    /// and the second also writes `start`.
    fn call_word_function(
        &self,
        name: &str,
        args: &[BasicMetadataValueEnum<'ctx>],
        memory: u64,
    ) -> IntValue<'ctx> {
        let function = self.module.get_function(name).unwrap_or_else(|| {
            let (ptr_type, i64_type) = (
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i64_type().into(),
            );
            // `sisu_words_count` takes the first two of these, `sisu_word_next` all four.
            let params: [BasicMetadataTypeEnum; 4] = [ptr_type, i64_type, i64_type, ptr_type];
            let function = self.module.add_function(
                name,
                self.context
                    .i64_type()
                    .fn_type(&params[..args.len()], false),
                None,
            );
            add_nounwind_memory(&self.module, function, memory);
            function
        });
        self.builder
            .build_call(function, args, "word")
            .expect(POSITIONED)
            .try_as_basic_value()
            .basic()
            .expect("a `sisu_word*` function should return an `i64`, as the runtime declares it")
            .into_int_value()
    }

    /// `FromChars`: sizes the string by `sisu_utf8_len`, allocates it, and has
    /// `sisu_utf8_encode` fill it. The `[char]` is released after.
    pub(super) fn string_from_chars(&mut self, base: &Expr) -> PointerValue<'ctx> {
        let (array, borrowed) = self.borrow_pointer(base);
        let zero = self.context.i64_type().const_zero();
        let (chars, count) = (
            self.element_ptr(&Type::Char, array, zero),
            self.array_len(array),
        );
        let len = self.utf8_size("sisu_utf8_len", chars, count);
        let string = self.new_string(len);
        self.utf8_fill(
            "sisu_utf8_encode",
            chars,
            count,
            self.string_byte(string, zero),
        );
        self.store(self.string_field(string, LEN), len.into());
        if !borrowed {
            self.release(array.into(), &base.ty);
        }
        string
    }

    /// A call of the runtime's `name(text, n) -> i64`, which sizes the `n` units at `text`;
    /// declared on first use, so that a module without these nodes stays as it was.
    fn utf8_size(&self, name: &str, text: PointerValue<'ctx>, n: IntValue<'ctx>) -> IntValue<'ctx> {
        let i64_type = self.context.i64_type();
        let function = self.module.get_function(name).unwrap_or_else(|| {
            let ptr_type = self.context.ptr_type(AddressSpace::default());
            let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
            self.module.add_function(name, fn_type, None)
        });
        self.builder
            .build_call(function, &[text.into(), n.into()], "size")
            .expect(POSITIONED)
            .try_as_basic_value()
            .basic()
            .expect("a `sisu_utf8_*` sizing function should return an `i64`, as the runtime declares it")
            .into_int_value()
    }

    /// A call of the runtime's `name(text, n, out)`, which converts the `n` units at `text` into
    /// `out`; declared on first use, as `utf8_size` is.
    fn utf8_fill(
        &self,
        name: &str,
        text: PointerValue<'ctx>,
        n: IntValue<'ctx>,
        out: PointerValue<'ctx>,
    ) {
        let function = self.module.get_function(name).unwrap_or_else(|| {
            let ptr_type = self.context.ptr_type(AddressSpace::default());
            let params = [
                ptr_type.into(),
                self.context.i64_type().into(),
                ptr_type.into(),
            ];
            let fn_type = self.context.void_type().fn_type(&params, false);
            self.module.add_function(name, fn_type, None)
        });
        self.call(function, &[text.into(), n.into(), out.into()]);
    }

    /// A call of the runtime's `name(value, out) -> len`, which writes the text of `value` at
    /// `out`; declared on first use, so that a module without interpolation stays as it was.
    fn write_text(
        &self,
        name: &str,
        value: BasicValueEnum<'ctx>,
        out: PointerValue<'ctx>,
    ) -> IntValue<'ctx> {
        // `zeroext` keeps the C `bool` ABI, as for `sisu_print_bool`.
        let bool_arg = value.get_type() == self.context.bool_type().into();
        let function = self.module.get_function(name).unwrap_or_else(|| {
            let ptr_type = self.context.ptr_type(AddressSpace::default());
            let fn_type = self
                .context
                .i64_type()
                .fn_type(&[value.get_type().into(), ptr_type.into()], false);
            let function = self.module.add_function(name, fn_type, None);
            if bool_arg {
                function.add_attribute(AttributeLoc::Param(0), zeroext(self.context));
            }
            function
        });
        let call = self
            .builder
            .build_call(function, &[value.into(), out.into()], "written")
            .expect(POSITIONED);
        if bool_arg {
            call.add_attribute(AttributeLoc::Param(0), zeroext(self.context));
        }
        call.try_as_basic_value()
            .basic()
            .expect("a `sisu_write_*` function should return an `i64`, as the runtime declares it")
            .into_int_value()
    }

    /// Copies the first `len` bytes of `string` to `to`.
    fn copy_bytes(&self, to: PointerValue<'ctx>, string: PointerValue<'ctx>, len: IntValue<'ctx>) {
        self.builder
            .build_memcpy(to, 1, self.string_field(string, BYTES), 1, len)
            .expect("an alignment of 1 should be a power of two");
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

    /// `print` or `eprint` of a string: `sisu_print_str(bytes, len)`, or `sisu_eprint_str`, then
    /// the release.
    pub(super) fn print_string(&mut self, stream: Stream, operand: &Expr) {
        let string = self.operand(operand).into_pointer_value();
        self.call(
            self.printers[stream as usize].str,
            &[
                self.string_field(string, BYTES).into(),
                self.string_len(string).into(),
            ],
        );
        self.release(string.into(), &operand.ty);
    }

    /// `panic(message)` at `span`: `sisu_panic_message(at, at_len, bytes, len)`, then
    /// `unreachable`. The message is not released: the process exits.
    pub(super) fn panic_string(&mut self, message: &Expr, span: Span) {
        let string = self.operand(message).into_pointer_value();
        let at = self.location(span);
        let at_ptr = self
            .builder
            .build_global_string_ptr(&at, "panic.at")
            .expect(POSITIONED);
        let at_len = self.context.i64_type().const_int(
            u64::try_from(at.len()).expect("a location should be shorter than 2^64 bytes"),
            false,
        );
        self.call(
            self.panic_message,
            &[
                at_ptr.as_pointer_value().into(),
                at_len.into(),
                self.string_field(string, BYTES).into(),
                self.string_len(string).into(),
            ],
        );
        self.builder.build_unreachable().expect(POSITIONED);
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
