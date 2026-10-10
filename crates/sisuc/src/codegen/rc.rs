//! Reference counting: retaining, releasing and comparing counted values, and the helpers
//! that do it. Each counted type gets one helper of each kind, declared on first use and
//! defined by `define_helpers` in one worklist pass.

use inkwell::basic_block::BasicBlock;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use super::Codegen;
use crate::tir::Type;

/// A kind of helper that a counted type has.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Helper {
    Release,
    Drop,
    Eq,
}

/// The type whose helpers a counted `ty` uses: a class or an array, with any `?` removed.
fn counted_core(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Class(_) => Some(ty),
        Type::Optional(payload) => counted_core(payload),
        _ => None,
    }
}

impl<'ctx> Codegen<'ctx, '_> {
    /// Declares `sisu_rc.<release|drop|eq>.<name>` on first use and queues its body; `name` is a
    /// class's name, or `[<element name>]` for an array (`[Node?]`, `[[i64]]`).
    pub(super) fn helper(&self, kind: Helper, ty: &Type) -> FunctionValue<'ctx> {
        let key = (kind, ty.clone());
        if let Some(function) = self.helpers.borrow().get(&key) {
            return *function;
        }
        let Type::Class(id) = ty else {
            unreachable!("only a class has helpers")
        };
        let name = &self.program.classes[id.0].name;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let takes_object = self.context.void_type().fn_type(&[ptr_type.into()], false);
        let (prefix, fn_type) = match kind {
            Helper::Release => ("release", takes_object),
            Helper::Drop => ("drop", takes_object),
            Helper::Eq => (
                "eq",
                self.context
                    .bool_type()
                    .fn_type(&[ptr_type.into(), ptr_type.into()], false),
            ),
        };
        let function = self
            .module
            .add_function(&format!("sisu_rc.{prefix}.{name}"), fn_type, None);
        self.helpers.borrow_mut().insert(key.clone(), function);
        self.pending.borrow_mut().push(key);
        function
    }

    /// Defines the counting helpers, which take their object without taking ownership:
    /// `@sisu_rc.retain` adds 1 to its count; `@sisu_rc.release.<C>` subtracts 1 and at 0
    /// calls `@sisu_rc.drop.<C>`, which releases each counted field, then frees the object.
    /// Both return at once on `null`. Drop does not release the chain field: it loads it,
    /// frees the object, subtracts 1 from the next object's count and at 0 drops it in a loop.
    /// `@sisu_rc.eq.<C>` is `==` on two objects; see `define_eq`. Defining a body can declare
    /// more helpers, so this runs until none is left to define.
    pub(super) fn define_helpers(&self) {
        let (object, done) = self.helper_entry(self.rc_retain);
        self.add_to_count(object, self.context.i64_type().const_int(1, false));
        self.branch_to(done);
        loop {
            let next = self.pending.borrow_mut().pop();
            let Some((kind, ty)) = next else {
                break;
            };
            let function = self.helpers.borrow()[&(kind, ty.clone())];
            match kind {
                Helper::Release => self.define_release(function, &ty),
                Helper::Drop => self.define_drop(function, &ty),
                Helper::Eq => self.define_eq(function, &ty),
            }
        }
    }

    /// `@sisu_rc.release.<C>`: subtracts 1 from the count, and at 0 drops the object.
    fn define_release(&self, function: FunctionValue<'ctx>, ty: &Type) {
        let (object, done) = self.helper_entry(function);
        let drop_block = self.context.append_basic_block(function, "drop");
        self.decrement(object, drop_block, done);
        self.builder.position_at_end(drop_block);
        self.call(self.helper(Helper::Drop, ty), &[object.into()]);
        self.branch_to(done);
    }

    /// `@sisu_rc.drop.<C>`: releases each counted field but the chain field, frees the
    /// object, then drops the chain's next object in a loop when its count reaches 0.
    fn define_drop(&self, drop: FunctionValue<'ctx>, ty: &Type) {
        let Type::Class(id) = ty else {
            unreachable!("only a class has helpers")
        };
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let entry = self.context.append_basic_block(drop, "entry");
        self.builder.position_at_end(entry);
        let chain = self.program.chain_field(*id);
        // With a chain field, the body is a loop over the chain's objects, in one frame.
        let looped = chain.map(|_| {
            let body = self.context.append_basic_block(drop, "loop");
            self.branch_to(body);
            self.builder.position_at_end(body);
            let object = self
                .builder
                .build_phi(ptr_type, "object")
                .expect("builder is positioned");
            object.add_incoming(&[(&param(drop), entry)]);
            (body, object)
        });
        let object = looped.map_or_else(
            || param(drop),
            |(_, phi)| phi.as_basic_value().into_pointer_value(),
        );
        let load_field = |index| {
            self.builder
                .build_load(ptr_type, self.field_ptr(ty, object, index), "field")
                .expect("builder is positioned")
        };
        for (index, field) in self.program.classes[id.0].fields.iter().enumerate() {
            if field.ty.is_counted() && chain != Some(index) {
                self.release(load_field(index), &field.ty);
            }
        }
        // Loaded before the free, which ends the object.
        let next = chain.map(|index| load_field(index).into_pointer_value());
        let (size, align) = self.class_layout(*id);
        let i64_type = self.context.i64_type();
        self.call(
            self.free,
            &[
                object.into(),
                i64_type.const_int(size, false).into(),
                i64_type.const_int(u64::from(align), false).into(),
            ],
        );
        if let (Some(next), Some((body, phi))) = (next, looped) {
            let done = self.context.append_basic_block(drop, "done");
            let counted = self.context.append_basic_block(drop, "next");
            let null = self
                .builder
                .build_is_null(next, "null")
                .expect("builder is positioned");
            self.builder
                .build_conditional_branch(null, done, counted)
                .expect("builder is positioned");
            self.builder.position_at_end(counted);
            self.decrement(next, body, done);
            phi.add_incoming(&[(&next, counted)]);
            self.builder.position_at_end(done);
        }
        self.builder
            .build_return(None)
            .expect("builder is positioned");
    }

    /// `@sisu_rc.eq.<C>(a, b)`: `true` when `a` and `b` are the same pointer (two `null`s
    /// included), `false` when one is `null`, otherwise whether each pair of fields is equal,
    /// compared in order up to the first difference. It neither retains nor releases.
    fn define_eq(&self, function: FunctionValue<'ctx>, ty: &Type) {
        let Type::Class(id) = ty else {
            unreachable!("only a class has helpers")
        };
        let block = |name| self.context.append_basic_block(function, name);
        let (entry, differ, objects) = (block("entry"), block("differ"), block("objects"));
        let (equal, unequal) = (block("equal"), block("unequal"));
        let bool_type = self.context.bool_type();
        for (at, value) in [(equal, 1), (unequal, 0)] {
            self.builder.position_at_end(at);
            self.builder
                .build_return(Some(&bool_type.const_int(value, false)))
                .expect("builder is positioned");
        }
        let nth = |n| {
            function
                .get_nth_param(n)
                .expect("eq takes two objects")
                .into_pointer_value()
        };
        let (a, b) = (nth(0), nth(1));
        let branch = |cond, then, other| {
            self.builder
                .build_conditional_branch(cond, then, other)
                .expect("builder is positioned");
        };
        self.builder.position_at_end(entry);
        branch(self.same_object(a, b), equal, differ);
        self.builder.position_at_end(differ);
        let null = |object| {
            self.builder
                .build_is_null(object, "null")
                .expect("builder is positioned")
        };
        let either_null = self
            .builder
            .build_or(null(a), null(b), "either_null")
            .expect("builder is positioned");
        branch(either_null, unequal, objects);
        self.builder.position_at_end(objects);
        for (index, field) in self.program.classes[id.0].fields.iter().enumerate() {
            let llvm_ty = self
                .llvm_type(&field.ty)
                .expect("checked: a field has a value type");
            let load = |object| {
                self.builder
                    .build_load(llvm_ty, self.field_ptr(ty, object, index), "field")
                    .expect("builder is positioned")
            };
            let next = block("next");
            branch(self.equal(&field.ty, load(a), load(b)), next, unequal);
            self.builder.position_at_end(next);
        }
        self.branch_to(equal);
    }

    /// Whether `lhs` and `rhs`, of type `ty`, are equal: `==` without releasing them.
    pub(super) fn equal(
        &self,
        ty: &Type,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
    ) -> IntValue<'ctx> {
        if let Some(core) = counted_core(ty) {
            return self
                .builder
                .build_call(
                    self.helper(Helper::Eq, core),
                    &[lhs.into(), rhs.into()],
                    "eq",
                )
                .expect("builder is positioned")
                .try_as_basic_value()
                .basic()
                .expect("eq returns a bool")
                .into_int_value();
        }
        if !matches!(ty, Type::Optional(_)) {
            return self.int_compare(IntPredicate::EQ, lhs.into_int_value(), rhs.into_int_value());
        }
        // `fa == fb && (!fa || va == vb)` on flags `f` and values `v`, with no branch.
        let part = |pair: BasicValueEnum<'ctx>, index| {
            self.builder
                .build_extract_value(pair.into_struct_value(), index, "part")
                .expect("a scalar optional is a pair")
                .into_int_value()
        };
        let (fa, fb) = (part(lhs, 0), part(rhs, 0));
        let flags = self.int_compare(IntPredicate::EQ, fa, fb);
        let values = self.int_compare(IntPredicate::EQ, part(lhs, 1), part(rhs, 1));
        let none = self
            .builder
            .build_not(fa, "none")
            .expect("builder is positioned");
        let values = self
            .builder
            .build_or(none, values, "values")
            .expect("builder is positioned");
        self.builder
            .build_and(flags, values, "equal")
            .expect("builder is positioned")
    }

    /// Subtracts 1 from the count of `object`, then branches to `at_zero` if it is 0, else to
    /// `otherwise`.
    fn decrement(
        &self,
        object: PointerValue<'ctx>,
        at_zero: BasicBlock<'ctx>,
        otherwise: BasicBlock<'ctx>,
    ) {
        let minus_one = self
            .context
            .i64_type()
            .const_int((-1_i64).cast_unsigned(), false);
        let count = self.add_to_count(object, minus_one);
        let zero = self.int_compare(
            IntPredicate::EQ,
            count,
            self.context.i64_type().const_zero(),
        );
        self.builder
            .build_conditional_branch(zero, at_zero, otherwise)
            .expect("builder is positioned");
    }

    /// Starts helper `function`, which returns at once when its object is `null`. Leaves the
    /// builder in the block for a real object; returns the object and the returning block.
    fn helper_entry(
        &self,
        function: FunctionValue<'ctx>,
    ) -> (PointerValue<'ctx>, BasicBlock<'ctx>) {
        let entry = self.context.append_basic_block(function, "entry");
        let counted = self.context.append_basic_block(function, "object");
        let done = self.context.append_basic_block(function, "done");
        let object = param(function);
        self.builder.position_at_end(entry);
        let null = self
            .builder
            .build_is_null(object, "null")
            .expect("builder is positioned");
        self.builder
            .build_conditional_branch(null, done, counted)
            .expect("builder is positioned");
        self.builder.position_at_end(done);
        self.builder
            .build_return(None)
            .expect("builder is positioned");
        self.builder.position_at_end(counted);
        (object, done)
    }

    /// Adds `delta` to the count of `object`, element 0; returns the new count.
    pub(super) fn add_to_count(
        &self,
        object: PointerValue<'ctx>,
        delta: IntValue<'ctx>,
    ) -> IntValue<'ctx> {
        let i64_type = self.context.i64_type();
        let count = self
            .builder
            .build_load(i64_type, object, "count")
            .expect("builder is positioned")
            .into_int_value();
        let count = self
            .builder
            .build_int_add(count, delta, "count")
            .expect("builder is positioned");
        self.store(object, count.into());
        count
    }

    /// Adds 1 to the count of `value` when `ty` is counted.
    pub(super) fn retain(&self, value: BasicValueEnum<'ctx>, ty: &Type) {
        if ty.is_counted() {
            self.call(self.rc_retain, &[value.into()]);
        }
    }

    /// Releases the reference `value` holds when `ty` is counted.
    pub(super) fn release(&self, value: BasicValueEnum<'ctx>, ty: &Type) {
        if let Some(core) = counted_core(ty) {
            self.call(self.helper(Helper::Release, core), &[value.into()]);
        }
    }
}

/// The single parameter of a counting helper: the object.
fn param(function: FunctionValue<'_>) -> PointerValue<'_> {
    function
        .get_nth_param(0)
        .expect("a helper takes one object")
        .into_pointer_value()
}
