//! Arrays at run time. An array is one heap block: a header of two `i64`s, the count and
//! the length, then the elements. This module owns the layout, the array nodes and the
//! bodies of the array helpers, which `define_helpers` reaches through `define_array_helper`.

use inkwell::types::{BasicTypeEnum, StructType};
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use super::rc::{Helper, param};
use super::{Codegen, POSITIONED, Slot};
use crate::diagnostic::Span;
use crate::tir::{Expr, ExprKind, Place, Type};

/// Where an array's elements sit in its block, from the target's data layout.
pub(super) struct ArrayLayout<'ctx> {
    /// The LLVM type of one element.
    pub(super) element: BasicTypeEnum<'ctx>,
    /// The first element's byte offset: the header's size rounded up to the element's
    /// alignment.
    pub(super) offset: u64,
    /// The ABI size of one element, which is also the distance between two elements.
    pub(super) size: u64,
    /// The block's alignment: the larger of the header's and the element's.
    pub(super) align: u64,
}

/// The element type of `ty`, an array type.
fn element_of(ty: &Type) -> &Type {
    let Type::Array(element) = ty else {
        unreachable!("the checker should give an array node an array type")
    };
    element
}

/// Whether evaluating `index` leaves every local as it was, so a local array read before it
/// still holds the same block after it. A whitelist of node kinds that call nothing, bind
/// nothing and move nothing: a node kind added later is excluded until it is listed here.
fn leaves_locals(index: &Expr) -> bool {
    match &index.kind {
        ExprKind::Int(_) | ExprKind::Local(_) => true,
        ExprKind::Unary { operand, .. }
        | ExprKind::Convert(operand)
        | ExprKind::Len(operand)
        | ExprKind::Field { base: operand, .. } => leaves_locals(operand),
        ExprKind::Binary { lhs, rhs, .. } => leaves_locals(lhs) && leaves_locals(rhs),
        ExprKind::Compare { operands, .. } => operands.iter().all(leaves_locals),
        _ => false,
    }
}

impl<'ctx> Codegen<'ctx, '_> {
    /// The header: the count, then the length.
    fn header(&self) -> StructType<'ctx> {
        let i64_type = self.context.i64_type().into();
        self.context.struct_type(&[i64_type, i64_type], false)
    }

    pub(super) fn array_layout(&self, element: &Type) -> ArrayLayout<'ctx> {
        let header = self.header();
        let ty = self
            .llvm_type(element)
            .expect("the checker should reject an array of `unit` or `never`, so its element has a value type");
        let align = u64::from(self.target_data.get_abi_alignment(&ty));
        ArrayLayout {
            element: ty,
            offset: self
                .target_data
                .get_abi_size(&header)
                .next_multiple_of(align),
            size: self.target_data.get_abi_size(&ty),
            align: u64::from(self.target_data.get_abi_alignment(&header)).max(align),
        }
    }

    /// A block with count 1 and length `len`. A literal passes a constant length, and `unique`
    /// a run-time length that already passed the checks; `[v; n]` gets them at `span`.
    pub(super) fn alloc_array(
        &self,
        element: &Type,
        len: IntValue<'ctx>,
        span: Option<Span>,
    ) -> PointerValue<'ctx> {
        let i64_type = self.context.i64_type();
        let layout = self.array_layout(element);
        let (size, offset) = (
            i64_type.const_int(layout.size, false),
            i64_type.const_int(layout.offset, false),
        );
        let total = if let Some(span) = span {
            let negative = self.int_compare(IntPredicate::SLT, len, i64_type.const_zero());
            self.panic_if(negative, span, "array length is negative");
            let (elements, wrapped) = self.overflowing("llvm.umul.with.overflow", len, size);
            let (total, carried) = self.overflowing("llvm.uadd.with.overflow", elements, offset);
            // `Layout::from_size_align` needs the size rounded up to the alignment to fit in
            // an `isize`.
            let largest = i64_type.const_int(i64::MAX.cast_unsigned() - layout.align + 1, false);
            let above = self.int_compare(IntPredicate::UGT, total, largest);
            let too_large = [wrapped, carried, above]
                .into_iter()
                .reduce(|a, b| self.builder.build_or(a, b, "too_large").expect(POSITIONED))
                .expect("the size checks should have three flags");
            self.panic_if(too_large, span, "array is too large");
            total
        } else {
            self.block_size(&layout, len)
        };
        let array = self
            .builder
            .build_call(
                self.alloc,
                &[total.into(), i64_type.const_int(layout.align, false).into()],
                "array",
            )
            .expect(POSITIONED)
            .try_as_basic_value()
            .basic()
            .expect("`sisu_alloc` should return a pointer, as the runtime declares it")
            .into_pointer_value();
        self.store(array, i64_type.const_int(1, false).into());
        self.store(self.len_ptr(array), len.into());
        array
    }

    /// `offset + len * size`: the size of a block whose length `len` passed the size checks,
    /// or is a literal's.
    fn block_size(&self, layout: &ArrayLayout<'ctx>, len: IntValue<'ctx>) -> IntValue<'ctx> {
        let i64_type = self.context.i64_type();
        let elements = self
            .builder
            .build_int_nuw_mul(len, i64_type.const_int(layout.size, false), "elements")
            .expect(POSITIONED);
        self.builder
            .build_int_nuw_add(elements, i64_type.const_int(layout.offset, false), "size")
            .expect(POSITIONED)
    }

    /// The address of the header's length.
    fn len_ptr(&self, array: PointerValue<'ctx>) -> PointerValue<'ctx> {
        self.builder
            .build_struct_gep(self.header(), array, 1, "len")
            .expect("the array header should have the length at index 1")
    }

    pub(super) fn array_len(&self, array: PointerValue<'ctx>) -> IntValue<'ctx> {
        self.builder
            .build_load(self.context.i64_type(), self.len_ptr(array), "len")
            .expect(POSITIONED)
            .into_int_value()
    }

    #[expect(
        unsafe_code,
        reason = "inkwell marks GEP unsafe for a wrong index list; each GEP here takes one `i64` index"
    )]
    pub(super) fn element_ptr(
        &self,
        element: &Type,
        array: PointerValue<'ctx>,
        index: IntValue<'ctx>,
    ) -> PointerValue<'ctx> {
        let layout = self.array_layout(element);
        let i64_type = self.context.i64_type();
        let offset = i64_type.const_int(layout.offset, false);
        // SAFETY: each GEP takes one `i64` index, which steps over whole `i8` or element
        // values and never indexes into an aggregate, so the index list is valid for its
        // pointee type. `inbounds` is sound because the block holds the header and then its
        // length's elements, `offset` is where the elements start, and every caller passes
        // an index below the length: `check_bounds` proved it, a loop counts up to it, or
        // `array_lit` stores at the constant indexes `0..n` of a block it just sized to `n`.
        // `unique` passes 0, whose address is at most one past the block's end, which
        // `inbounds` allows.
        unsafe {
            let elements = self
                .builder
                .build_in_bounds_gep(self.context.i8_type(), array, &[offset], "elements")
                .expect(POSITIONED);
            self.builder
                .build_in_bounds_gep(layout.element, elements, &[index], "element")
                .expect(POSITIONED)
        }
    }

    /// The element at `index`, without retaining it.
    fn load_element(
        &self,
        element: &Type,
        array: PointerValue<'ctx>,
        index: IntValue<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        let layout = self.array_layout(element);
        self.builder
            .build_load(
                layout.element,
                self.element_ptr(element, array, index),
                "element",
            )
            .expect(POSITIONED)
    }

    /// Panics at `span` with `sisu_panic_bounds` when `index uge len`, which is
    /// `0 <= index < len` negated: one unsigned compare also catches a negative index.
    pub(super) fn check_bounds(
        &self,
        array: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        span: Span,
    ) {
        let len = self.array_len(array);
        let outside = self.int_compare(IntPredicate::UGE, index, len);
        self.panic_with(
            outside,
            &self.location(span),
            self.panic_bounds,
            &[index.into(), len.into()],
        );
    }

    /// Runs `body` for each index from 0 up to `len`, which is not negative. `body` may leave
    /// the builder in another block; the loop goes on from there.
    fn count_loop(&self, len: IntValue<'ctx>, mut body: impl FnMut(IntValue<'ctx>)) {
        let function = self.current_function();
        let before = self.current_block();
        let head = self.context.append_basic_block(function, "array.head");
        let step = self.context.append_basic_block(function, "array.body");
        let done = self.context.append_basic_block(function, "array.done");
        self.branch_to(head);
        self.builder.position_at_end(head);
        let i64_type = self.context.i64_type();
        let phi = self.builder.build_phi(i64_type, "i").expect(POSITIONED);
        phi.add_incoming(&[(&i64_type.const_zero(), before)]);
        let index = phi.as_basic_value().into_int_value();
        let more = self.int_compare(IntPredicate::ULT, index, len);
        self.builder
            .build_conditional_branch(more, step, done)
            .expect(POSITIONED);
        self.builder.position_at_end(step);
        body(index);
        let next = self
            .builder
            .build_int_nuw_add(index, i64_type.const_int(1, false), "next")
            .expect(POSITIONED);
        phi.add_incoming(&[(&next, self.current_block())]);
        self.branch_to(head);
        self.builder.position_at_end(done);
    }

    /// `ArrayLit`: every element first, left to right, then the block, which takes each
    /// element's reference. Allocating last means an element that exits leaves no block.
    pub(super) fn array_lit(&mut self, ty: &Type, elements: &[Expr]) -> PointerValue<'ctx> {
        let values: Vec<_> = elements
            .iter()
            .map(|e| {
                self.expr(e).expect(
                    "the checker should give every array element a value type, never `unit` or `never`",
                )
            })
            .collect();
        let element = element_of(ty);
        let i64_type = self.context.i64_type();
        let len = u64::try_from(values.len())
            .expect("an array literal should have fewer than 2^64 elements");
        let array = self.alloc_array(element, i64_type.const_int(len, false), None);
        for (index, value) in (0..).zip(values) {
            let slot = self.element_ptr(element, array, i64_type.const_int(index, false));
            self.store(slot, value);
        }
        array
    }

    /// `ArrayFill`: `value`, then `len`, then the block, with `value` in every slot. A
    /// counted `value` gains `len - 1` references in one step, or loses its one when `len`
    /// is 0.
    pub(super) fn array_fill(&mut self, ty: &Type, value: &Expr, len: &Expr) -> PointerValue<'ctx> {
        let element = element_of(ty);
        let filler = self.expr(value).expect(
            "the checker should give the value of `[v; n]` a value type, never `unit` or `never`",
        );
        let count = self.value(len);
        let array = self.alloc_array(element, count, Some(len.span));
        self.count_loop(count, |index| {
            self.store(self.element_ptr(element, array, index), filler);
        });
        if element.is_counted() {
            self.share_filler(filler, element, count);
        }
        array
    }

    /// Gives `filler`, stored into `count` slots, one reference per slot: adds `count - 1`
    /// when it is an object or array and `count > 0`, or releases it when `count` is 0.
    fn share_filler(&self, filler: BasicValueEnum<'ctx>, ty: &Type, count: IntValue<'ctx>) {
        let i64_type = self.context.i64_type();
        let function = self.current_function();
        let block = |name| self.context.append_basic_block(function, name);
        let (empty, nonzero, shared, done) = (
            block("fill.empty"),
            block("fill.nonzero"),
            block("fill.share"),
            block("fill.done"),
        );
        let zero = self.int_compare(IntPredicate::EQ, count, i64_type.const_zero());
        self.builder
            .build_conditional_branch(zero, empty, nonzero)
            .expect(POSITIONED);
        self.builder.position_at_end(empty);
        self.release(filler, ty);
        self.branch_to(done);
        self.builder.position_at_end(nonzero);
        let object = filler.into_pointer_value();
        let null = self
            .builder
            .build_is_null(object, "null")
            .expect(POSITIONED);
        self.builder
            .build_conditional_branch(null, done, shared)
            .expect(POSITIONED);
        self.builder.position_at_end(shared);
        self.check_count(object, 0);
        let extra = self
            .builder
            .build_int_sub(count, i64_type.const_int(1, false), "extra")
            .expect(POSITIONED);
        self.add_to_count(object, extra);
        self.branch_to(done);
        self.builder.position_at_end(done);
    }

    /// `Index`: `base`, then `index`, the bounds check, then the element, retained. The base
    /// is released after the load, unless it is a local read whose index leaves every local
    /// as it was: the local then keeps the block alive, so codegen skips its retain and
    /// release.
    pub(super) fn index(&mut self, ty: &Type, base: &Expr, index: &Expr) -> BasicValueEnum<'ctx> {
        let local = match base.kind {
            ExprKind::Local(local) if leaves_locals(index) => Some(local),
            _ => None,
        };
        let array = match local {
            Some(local) => self.load_local(local),
            None => self.expr(base),
        }
        .expect("the checker should only index an array, which lowers to a pointer")
        .into_pointer_value();
        let at = self.value(index);
        self.check_bounds(array, at, index.span);
        let value = self.load_element(ty, array, at);
        self.retain(value, ty);
        if local.is_none() {
            self.release(array.into(), &base.ty);
        }
        value
    }

    /// `Len`: the length of `base`, which is then released.
    pub(super) fn len(&mut self, base: &Expr) -> IntValue<'ctx> {
        let array = self
            .expr(base)
            .expect("the checker should only call `len` on an array, which lowers to a pointer")
            .into_pointer_value();
        let len = self.array_len(array);
        self.release(array.into(), &base.ty);
        len
    }

    /// `Sum`: the elements of `base`, an `[i64]`, added in order with the checked add of `+`,
    /// which panics at `span`; then the base is released.
    pub(super) fn sum(&mut self, base: &Expr, span: Span) -> IntValue<'ctx> {
        let array = self
            .expr(base)
            .expect("the checker should only call `sum` on an array, which lowers to a pointer")
            .into_pointer_value();
        let i64_type = self.context.i64_type();
        let total = self.entry_alloca(i64_type.into(), "total");
        self.store(total, i64_type.const_zero().into());
        self.count_loop(self.array_len(array), |index| {
            let element = self.load_element(&Type::I64, array, index).into_int_value();
            let so_far = self.load(i64_type.into(), total).into_int_value();
            self.store(
                total,
                self.checked("llvm.sadd.with.overflow", so_far, element, span)
                    .into(),
            );
        });
        let sum = self.load(i64_type.into(), total).into_int_value();
        self.release(array.into(), &base.ty);
        sum
    }

    /// `Contains`: `base`, then `value`; whether some element of `base` is `==` to `value`,
    /// stopping at the first that is. Then both are released.
    pub(super) fn contains(&mut self, base: &Expr, value: &Expr) -> IntValue<'ctx> {
        let element = element_of(&base.ty);
        let array = self
            .expr(base)
            .expect(
                "the checker should only call `contains` on an array, which lowers to a pointer",
            )
            .into_pointer_value();
        let needle = self.operand(value);
        let function = self.current_function();
        let found = self.context.append_basic_block(function, "contains.found");
        let end = self.context.append_basic_block(function, "contains.end");
        self.count_loop(self.array_len(array), |index| {
            let same = self.equal(element, self.load_element(element, array, index), needle);
            let next = self.context.append_basic_block(function, "contains.next");
            self.builder
                .build_conditional_branch(same, found, next)
                .expect(POSITIONED);
            self.builder.position_at_end(next);
        });
        let missed = self.current_block();
        self.branch_to(end);
        self.builder.position_at_end(found);
        self.branch_to(end);
        self.builder.position_at_end(end);
        let bool_type = self.context.bool_type();
        let hit = self
            .phi(
                bool_type,
                &[
                    (bool_type.const_zero(), missed),
                    (bool_type.const_int(1, false), found),
                ],
            )
            .into_int_value();
        self.release(array.into(), &base.ty);
        self.release(needle, &value.ty);
        hit
    }

    /// `Assign { place: Index { .. }, value }`. The place's own operands come first, left to
    /// right: a field root's base, then each index from the root outwards. Then `value`, so
    /// that a value which stores the array elsewhere makes the walk copy it. Then the walk from
    /// the root's slot: at each level, load the array, check the index against its length,
    /// make the array unique, and take the element's address as the next slot. Checking first
    /// means a level whose index is out of bounds copies nothing; an outer level may already
    /// have copied. Last, the store, the release of the old
    /// element, and the release of a field root's base.
    pub(super) fn assign_index(&mut self, place: &Place, value: &Expr) {
        let mut indexes = Vec::new();
        let mut root = place;
        while let Place::Index { base, index } = root {
            indexes.push(index);
            root = base;
        }
        indexes.reverse();
        let object = match root {
            Place::Field { base, .. } => Some((
                self.expr(base)
                    .expect("the checker should only make a field place on a class-typed base, which lowers to a pointer")
                    .into_pointer_value(),
                &base.ty,
            )),
            _ => None,
        };
        // An operand that exits ends the statement. `spill` bound every counted operand
        // before it to a local, so nothing is left to release here.
        let mut at = Vec::with_capacity(indexes.len());
        for index in &indexes {
            let Some(i) = self.expr(index) else { return };
            at.push(i.into_int_value());
        }
        let Some(value) = self.expr(value) else {
            return;
        };
        let (mut slot, mut ty) = match root {
            Place::Local(local) => {
                let Some(Slot::Alloca(ptr, _)) = self.locals[local.0] else {
                    unreachable!(
                        "the checker should only let a variable be written, and every variable has an `alloca`"
                    )
                };
                (ptr, &self.types[local.0])
            }
            Place::Field { base, index } => {
                let Type::Class(class) = base.ty else {
                    unreachable!("the checker should only make a field place on a class-typed base")
                };
                let (object, _) =
                    object.expect("a field root's base should have been evaluated before the walk");
                (
                    self.field_ptr(&base.ty, object, *index),
                    &self.program.classes[class.0].fields[*index].ty,
                )
            }
            Place::Index { .. } => unreachable!("the loop above should stop at the root"),
        };
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        for (index, at) in indexes.iter().zip(at) {
            let array = self
                .builder
                .build_load(ptr_type, slot, "array")
                .expect(POSITIONED)
                .into_pointer_value();
            self.check_bounds(array, at, index.span);
            let array = self.unique(slot, ty);
            ty = element_of(ty);
            slot = self.element_ptr(ty, array, at);
        }
        let old = ty.is_counted().then(|| self.load(value.get_type(), slot));
        self.store(slot, value);
        self.discard(old, ty);
        if let Some((object, ty)) = object {
            self.release(object.into(), ty);
        }
    }

    /// The body of `@sisu_rc.drop.[T]`, `@sisu_rc.eq.[T]` or `@sisu_rc.unique.[T]`, `T`
    /// being `element`.
    pub(super) fn define_array_helper(
        &self,
        kind: Helper,
        element: &Type,
        function: FunctionValue<'ctx>,
    ) {
        match kind {
            Helper::Drop => self.define_array_drop(element, function),
            Helper::Eq => self.define_array_eq(element, function),
            Helper::Unique => self.define_array_unique(element, function),
            Helper::Release => {
                unreachable!("`define_helpers` should define every `release` itself")
            }
        }
    }

    /// `@sisu_rc.drop.[T]`: releases each counted element in order, then frees the block, its
    /// size recomputed from the length.
    fn define_array_drop(&self, element: &Type, function: FunctionValue<'ctx>) {
        self.builder
            .position_at_end(self.context.append_basic_block(function, "entry"));
        let array = param(function);
        let len = self.array_len(array);
        if element.is_counted() {
            self.count_loop(len, |index| {
                self.release(self.load_element(element, array, index), element);
            });
        }
        let layout = self.array_layout(element);
        let i64_type = self.context.i64_type();
        self.call(
            self.free,
            &[
                array.into(),
                self.block_size(&layout, len).into(),
                i64_type.const_int(layout.align, false).into(),
            ],
        );
        self.builder.build_return(None).expect(POSITIONED);
    }

    /// `@sisu_rc.eq.[T](a, b)`: see `eq_entry` for the same pointer and `null`; otherwise the
    /// lengths, then each pair of elements by `T`'s `==`, up to the first difference.
    fn define_array_eq(&self, element: &Type, function: FunctionValue<'ctx>) {
        let (a, b, equal, unequal) = self.eq_entry(function);
        let len = self.array_len(a);
        let same_len = self.int_compare(IntPredicate::EQ, len, self.array_len(b));
        let elements = self.context.append_basic_block(function, "elements");
        self.builder
            .build_conditional_branch(same_len, elements, unequal)
            .expect(POSITIONED);
        self.builder.position_at_end(elements);
        self.count_loop(len, |index| {
            let pair_equal = self.equal(
                element,
                self.load_element(element, a, index),
                self.load_element(element, b, index),
            );
            let next = self.context.append_basic_block(function, "next");
            self.builder
                .build_conditional_branch(pair_equal, next, unequal)
                .expect(POSITIONED);
            self.builder.position_at_end(next);
        });
        self.branch_to(equal);
    }

    /// `@sisu_rc.unique.[T](slot) -> ptr`: the array in `slot` when its count is 1. Otherwise
    /// a new block of the same length with count 1, the elements copied with `llvm.memcpy`
    /// and each counted one retained, stored into `slot`; the old block is released. It has no
    /// `null` early return: the slot is an `alloca`, a field address or an element address.
    fn define_array_unique(&self, element: &Type, function: FunctionValue<'ctx>) {
        let block = |name| self.context.append_basic_block(function, name);
        let (entry, only, shared) = (block("entry"), block("only"), block("shared"));
        self.builder.position_at_end(entry);
        let slot = param(function);
        let i64_type = self.context.i64_type();
        let array = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                slot,
                "array",
            )
            .expect(POSITIONED)
            .into_pointer_value();
        // A dead block, count below 1, would otherwise pass for a shared one and be copied.
        self.check_count(array, 1);
        let count = self
            .builder
            .build_load(i64_type, array, "count")
            .expect(POSITIONED)
            .into_int_value();
        let one = self.int_compare(IntPredicate::EQ, count, i64_type.const_int(1, false));
        self.builder
            .build_conditional_branch(one, only, shared)
            .expect(POSITIONED);
        self.builder.position_at_end(only);
        self.builder.build_return(Some(&array)).expect(POSITIONED);
        self.builder.position_at_end(shared);
        let len = self.array_len(array);
        // The length fit once, so the new block needs no size check.
        let copy = self.alloc_array(element, len, None);
        let layout = self.array_layout(element);
        let bytes = self
            .builder
            .build_int_nuw_mul(len, i64_type.const_int(layout.size, false), "bytes")
            .expect(POSITIONED);
        // The elements start at an offset that is a multiple of the block's alignment.
        let align = u32::try_from(layout.align)
            .expect("the block's alignment should fit in a `u32`, as each ABI alignment does");
        let zero = i64_type.const_zero();
        self.builder
            .build_memcpy(
                self.element_ptr(element, copy, zero),
                align,
                self.element_ptr(element, array, zero),
                align,
                bytes,
            )
            .expect("the block's alignment should be a power of two, as each ABI alignment is");
        if element.is_counted() {
            self.count_loop(len, |index| {
                self.retain(self.load_element(element, copy, index), element);
            });
        }
        self.store(slot, copy.into());
        // Releasing the old block never frees it: its count was above 1.
        self.add_to_count(array, i64_type.const_int((-1_i64).cast_unsigned(), false));
        self.builder.build_return(Some(&copy)).expect(POSITIONED);
    }
}
