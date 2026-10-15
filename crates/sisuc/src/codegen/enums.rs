//! Enums at run time. An enum value is a tag and its variant's payload, held inline:
//! `%sisu.<Name> = { tag, [n x iA] }`, with one struct per variant, `%sisu.<Name>.<Variant>`,
//! laid over the payload storage. This module owns the layouts, building a variant, and the
//! bodies of the enum helpers, which `define_helpers` reaches through `define_enum_helper`.

use std::num::NonZeroU32;

use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::basic_block::BasicBlock;
use inkwell::types::{IntType, StructType};
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};

use super::rc::{Helper, param};
use super::{Codegen, POSITIONED};
use crate::tir::{Enum, EnumId, Expr, Payload, Type};

/// An enum's LLVM types.
pub(super) struct EnumLayout<'ctx> {
    /// `{ tag, [n x iA] }`, or `{ tag }` when no variant has a field. `iA` is the integer type
    /// as wide as the largest payload ABI alignment `a`, and `n` the largest payload ABI size
    /// rounded up to whole `a`s, so the storage fits every payload at its alignment.
    pub(super) ty: StructType<'ctx>,
    /// `i8`, `i16` or `i32`, the smallest that holds the variant count. Tag `v` is variant `v`.
    pub(super) tag: IntType<'ctx>,
    /// Indexed by variant: `{ fields... }`, a field's LLVM type being its type's, or `ptr` when
    /// boxed.
    pub(super) payloads: Vec<StructType<'ctx>>,
}

/// The enums `e` holds inline: each unboxed payload field of an enum type or its optional.
fn held_inline(e: &Enum) -> impl Iterator<Item = EnumId> + '_ {
    e.variants
        .iter()
        .flat_map(|variant| &variant.fields)
        .filter(|field| !field.boxed)
        .filter_map(|field| match &field.ty {
            Type::Enum(id) => Some(*id),
            Type::Optional(payload) => match **payload {
                Type::Enum(id) => Some(id),
                _ => None,
            },
            _ => None,
        })
}

impl<'ctx> Codegen<'ctx, '_> {
    pub(super) fn enum_layout(&self, id: EnumId) -> &EnumLayout<'ctx> {
        &self.enums[id.0]
    }

    /// Builds every enum's layout: each enum's type and its payload structs, named first so
    /// that a payload can name any enum, then each payload's body, then each enum's body. An
    /// enum's body needs its payloads' sizes, so an enum held inline gets its body before its
    /// holder: post-order over the inline fields, with an explicit stack. The checker boxes
    /// every field that closes a cycle, so the inline fields form no cycle and the walk ends.
    pub(super) fn lay_out_enums(&mut self) {
        let program = self.program;
        let context = self.context;
        self.enums = program
            .enums
            .iter()
            .map(|e| {
                let count = e.variants.len();
                let tag = if count <= 1 << 8 {
                    context.i8_type()
                } else if count <= 1 << 16 {
                    context.i16_type()
                } else {
                    context.i32_type()
                };
                EnumLayout {
                    ty: context.opaque_struct_type(&format!("sisu.{}", e.name)),
                    tag,
                    payloads: e
                        .variants
                        .iter()
                        .map(|v| context.opaque_struct_type(&format!("sisu.{}.{}", e.name, v.name)))
                        .collect(),
                }
            })
            .collect();
        let ptr_type = context.ptr_type(AddressSpace::default());
        for (e, layout) in program.enums.iter().zip(&self.enums) {
            for (variant, payload) in e.variants.iter().zip(&layout.payloads) {
                let fields: Vec<_> = variant
                    .fields
                    .iter()
                    .map(|field| {
                        if field.boxed {
                            ptr_type.into()
                        } else {
                            self.llvm_type(&field.ty).expect(
                                "the checker should give every payload field a value type, never `unit` or `never`",
                            )
                        }
                    })
                    .collect();
                payload.set_body(&fields, false);
            }
        }
        let mut done = vec![false; program.enums.len()];
        for root in 0..program.enums.len() {
            // `(id, true)` once the enums `id` holds inline are queued before it.
            let mut stack = vec![(root, false)];
            while let Some((id, held_queued)) = stack.pop() {
                if done[id] {
                    continue;
                }
                if held_queued {
                    self.set_enum_body(id);
                    done[id] = true;
                    continue;
                }
                stack.push((id, true));
                stack.extend(held_inline(&program.enums[id]).map(|held| (held.0, false)));
            }
        }
    }

    /// Sets the body of enum `id`, whose payload structs are all sized.
    fn set_enum_body(&self, id: usize) {
        let layout = &self.enums[id];
        let (size, align) = layout
            .payloads
            .iter()
            .fold((0, 1), |(size, align), payload| {
                (
                    size.max(self.target_data.get_abi_size(payload)),
                    align.max(self.target_data.get_abi_alignment(payload)),
                )
            });
        let tag = layout.tag.into();
        if size == 0 {
            layout.ty.set_body(&[tag], false);
            return;
        }
        let unit = NonZeroU32::new(align * 8)
            .and_then(|bits| self.context.custom_width_int_type(bits).ok())
            .expect("an ABI alignment should be a positive number of bytes that LLVM has an integer type for");
        let units = u32::try_from(size.div_ceil(u64::from(align)))
            .expect("a payload should be smaller than 2^32 units of its alignment");
        layout
            .ty
            .set_body(&[tag, unit.array_type(units).into()], false);
    }

    /// The address of the tag of the enum `id` at `value_addr`.
    fn tag_ptr(&self, id: EnumId, value_addr: PointerValue<'ctx>) -> PointerValue<'ctx> {
        self.builder
            .build_struct_gep(self.enums[id.0].ty, value_addr, 0, "tag")
            .expect("an enum's struct should have the tag at element 0")
    }

    /// The tag of the enum `id` at `value_addr`.
    fn load_tag(&self, id: EnumId, value_addr: PointerValue<'ctx>) -> IntValue<'ctx> {
        self.builder
            .build_load(self.enums[id.0].tag, self.tag_ptr(id, value_addr), "tag")
            .expect(POSITIONED)
            .into_int_value()
    }

    /// The address of payload field `field` of variant `variant` in the enum `id` at
    /// `value_addr`, through the variant's struct laid over the payload storage.
    pub(super) fn payload_place(
        &self,
        value_addr: PointerValue<'ctx>,
        id: EnumId,
        variant: usize,
        field: usize,
    ) -> PointerValue<'ctx> {
        assert!(
            !self.program.enums[id.0].variants[variant].fields[field].boxed,
            "boxes land in Task 5"
        );
        let layout = &self.enums[id.0];
        let storage = self
            .builder
            .build_struct_gep(layout.ty, value_addr, 1, "payload")
            .expect("an enum with a payload field should have its payload storage at element 1");
        let index = u32::try_from(field).expect(
            "a variant should have fewer than 2^32 fields, since field indexes are `u32` in LLVM",
        );
        self.builder
            .build_struct_gep(layout.payloads[variant], storage, index, "field")
            .expect("the checker should only emit a payload field index that the variant has")
    }

    /// The value of payload field `field` of variant `variant` in the enum `id` at
    /// `value_addr`, borrowed: not retained.
    pub(super) fn payload_value(
        &self,
        value_addr: PointerValue<'ctx>,
        id: EnumId,
        variant: usize,
        field: usize,
    ) -> BasicValueEnum<'ctx> {
        let ty = &self.program.enums[id.0].variants[variant].fields[field].ty;
        let llvm_ty = self.llvm_type(ty).expect(
            "the checker should give every payload field a value type, never `unit` or `never`",
        );
        self.builder
            .build_load(
                llvm_ty,
                self.payload_place(value_addr, id, variant, field),
                "field",
            )
            .expect(POSITIONED)
    }

    /// `Variant`: each argument, left to right, then the value: the tag and each field stored
    /// into a scratch slot, which is loaded whole. The slot is zeroed first, so that the whole
    /// load never reads a byte that no store gave a value.
    pub(super) fn build_variant(
        &mut self,
        id: EnumId,
        variant: usize,
        args: &[Expr],
    ) -> BasicValueEnum<'ctx> {
        let values: Vec<_> = args.iter().map(|a| self.operand(a)).collect();
        let layout = self.enum_layout(id);
        let (ty, tag) = (layout.ty, layout.tag);
        let slot = self.entry_alloca(ty.into(), "variant");
        self.store(slot, ty.const_zero().into());
        let tag_value = u64::try_from(variant).expect("a variant index should fit in a `u64`");
        self.store(
            self.tag_ptr(id, slot),
            tag.const_int(tag_value, false).into(),
        );
        for (field, value) in values.into_iter().enumerate() {
            self.store(self.payload_place(slot, id, variant, field), value);
        }
        self.builder
            .build_load(ty, slot, "variant")
            .expect(POSITIONED)
    }

    /// A scratch slot holding the enum `value`, for a helper that takes its address.
    pub(super) fn enum_slot(&self, value: BasicValueEnum<'ctx>) -> PointerValue<'ctx> {
        let slot = self.entry_alloca(value.get_type(), "enum");
        self.store(slot, value);
        slot
    }

    /// The body of `@sisu_rc.retain.<E>`, `@sisu_rc.release.<E>` or `@sisu_rc.eq.<E>`.
    pub(super) fn define_enum_helper(
        &self,
        kind: Helper,
        id: EnumId,
        function: FunctionValue<'ctx>,
    ) {
        match kind {
            Helper::Retain | Helper::Release => self.define_enum_count(kind, id, function),
            Helper::Eq => self.define_enum_eq(id, function),
            Helper::Drop | Helper::Unique => {
                unreachable!(
                    "an enum should be queued only for `retain`, `release` and `eq` helpers"
                )
            }
        }
    }

    /// `@sisu_rc.retain.<E>(ptr)` or `@sisu_rc.release.<E>(ptr)`, as `kind` says: switches on
    /// the tag of the value at `ptr`, and retains or releases each counted field of its variant
    /// through `retain` or `release`. Releasing the zero value, which a `Move` leaves in a slot,
    /// releases nothing: it is variant 0 with every field zero, and `release` skips a `null`
    /// string, object or array, an optional whose flag is clear, and a zero enum in turn.
    fn define_enum_count(&self, kind: Helper, id: EnumId, function: FunctionValue<'ctx>) {
        let entry = self.context.append_basic_block(function, "entry");
        let done = self.context.append_basic_block(function, "done");
        self.builder.position_at_end(done);
        self.builder.build_return(None).expect(POSITIONED);
        self.builder.position_at_end(entry);
        let value = param(function);
        let enums = &self.program.enums;
        self.for_each_field(
            id,
            self.load_tag(id, value),
            done,
            |payload| payload.boxed || payload.ty.is_counted(enums),
            |variant, field, ty| {
                let field = self.payload_value(value, id, variant, field);
                self.retain_or_release(kind, field, ty);
            },
        );
    }

    /// `@sisu_rc.eq.<E>(a, b) -> i1`: `false` when the tags differ, else whether each pair of
    /// the variant's fields is `==`, stopping at the first that is not. It neither retains nor
    /// releases.
    fn define_enum_eq(&self, id: EnumId, function: FunctionValue<'ctx>) {
        let block = |name| self.context.append_basic_block(function, name);
        let (entry, fields, equal, unequal) = (
            block("entry"),
            block("fields"),
            block("equal"),
            block("unequal"),
        );
        let bool_type = self.context.bool_type();
        for (at, value) in [(equal, 1), (unequal, 0)] {
            self.builder.position_at_end(at);
            self.builder
                .build_return(Some(&bool_type.const_int(value, false)))
                .expect(POSITIONED);
        }
        let nth = |n| {
            function
                .get_nth_param(n)
                .expect("the generated `eq` helper should be declared with two parameters")
                .into_pointer_value()
        };
        let (a, b) = (nth(0), nth(1));
        self.builder.position_at_end(entry);
        let tag = self.load_tag(id, a);
        let same = self.int_compare(IntPredicate::EQ, tag, self.load_tag(id, b));
        self.builder
            .build_conditional_branch(same, fields, unequal)
            .expect(POSITIONED);
        self.builder.position_at_end(fields);
        self.for_each_field(
            id,
            tag,
            equal,
            |_| true,
            |variant, field, ty| {
                let pair = self.equal(
                    ty,
                    self.payload_value(a, id, variant, field),
                    self.payload_value(b, id, variant, field),
                );
                let next = block("next");
                self.builder
                    .build_conditional_branch(pair, next, unequal)
                    .expect(POSITIONED);
                self.builder.position_at_end(next);
            },
        );
    }

    /// Switches on `tag`, of enum `id`. Each variant with a field that `wanted` picks gets a
    /// block that runs `each(variant, field, type)` for each picked field in order, then
    /// branches to `done`; every other tag branches to `done` at once.
    fn for_each_field(
        &self,
        id: EnumId,
        tag: IntValue<'ctx>,
        done: BasicBlock<'ctx>,
        wanted: impl Fn(&Payload) -> bool,
        mut each: impl FnMut(usize, usize, &Type),
    ) {
        let function = self.current_function();
        let e = &self.program.enums[id.0];
        let arms: Vec<_> = (0_u64..)
            .zip(&e.variants)
            .enumerate()
            .filter(|(_, (_, variant))| variant.fields.iter().any(&wanted))
            .map(|(index, (value, variant))| {
                let block = self.context.append_basic_block(function, &variant.name);
                (index, self.enums[id.0].tag.const_int(value, false), block)
            })
            .collect();
        let cases: Vec<_> = arms
            .iter()
            .map(|&(_, value, block)| (value, block))
            .collect();
        self.builder
            .build_switch(tag, done, &cases)
            .expect(POSITIONED);
        for (variant, _, block) in arms {
            self.builder.position_at_end(block);
            for (field, payload) in e.variants[variant].fields.iter().enumerate() {
                if wanted(payload) {
                    each(variant, field, &payload.ty);
                }
            }
            self.branch_to(done);
        }
    }
}
