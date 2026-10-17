//! `match`. The scrutinee is evaluated once into `temp`, a local the `match` owns. The arms'
//! patterns are tested in order, in place against `temp`; a binding takes a borrowed copy of its
//! bits while the patterns and the guard run. An `|` is a choice point: a later failure, of a
//! sub-pattern or of the guard, retries the most recent choice point that has an alternative left,
//! which tries the alternatives in the order of their left-to-right expansion. The first arm whose
//! pattern and guard both hold commits: each binding adds 1, `temp` is released, and the body runs
//! with the bindings live. The arms' values join in a `phi`. After the last arm the `match` traps:
//! the checker proves that some arm matches, and a checker bug must not become undefined
//! behaviour.
//!
//! shortcut: bindings add 1 and the match then releases its scrutinee, so a part it held alone is
//! counted up and down once; move fields out when an enum benchmark shows that counting or the
//! box's free and allocation matter; box reuse depends on it (roadmap, "Reusing a box in match")

use inkwell::basic_block::BasicBlock;
use inkwell::intrinsics::Intrinsic;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use super::{Codegen, POSITIONED, Slot};
use crate::tir::{Arm, Expr, LocalId, Pattern, PatternKind, Type, TypeKind};

/// Where a tested value lives.
#[derive(Clone, Copy)]
enum At<'ctx> {
    /// In `temp`'s slot: the scrutinee itself.
    Root(PointerValue<'ctx>),
    /// At the address this slot holds: a payload field. Each use loads the address from the
    /// slot, so no block uses an address computed in another block.
    Field(PointerValue<'ctx>),
}

impl<'ctx> Codegen<'ctx, '_> {
    /// `match scrutinee { arms }`, of type `ty`; `None` for `unit`, or when no arm falls
    /// through. Every arm starts from the same live locals, those of the `match` and `temp`
    /// when it is counted. A committed arm's bindings sit on `live` below its body's block, so
    /// an exit from the body releases them, and the `match` releases them when the body falls
    /// through.
    pub(super) fn match_expr(
        &mut self,
        scrutinee: &Expr,
        temp: LocalId,
        arms: &[Arm],
        ty: Type,
    ) -> Option<BasicValueEnum<'ctx>> {
        let outer = self.live.len();
        let value = self.expr(scrutinee).expect(
            "the checker should reject a scrutinee that `match` cannot take, so it has a value",
        );
        let root = self.own_slot(temp, "scrutinee");
        self.store(root, value);
        let counted = scrutinee.ty.is_counted(&self.program.enums);
        let function = self.current_function();
        let mut ends = Vec::new();
        for arm in arms {
            self.live.truncate(outer);
            if counted {
                self.live.push(temp);
            }
            for &local in &arm.bindings {
                self.own_slot(local, "bind");
            }
            // The last arm's failure block is the trap.
            let fail = self.context.append_basic_block(function, "arm.next");
            let retry = self.test(&arm.pattern, At::Root(root), scrutinee.ty, fail);
            if let Some(guard) = &arm.guard {
                // The guard reads the bindings unretained: `temp` owns what they point into.
                let holds = self
                    .expr(guard)
                    .expect("the checker should give a guard a `bool` value")
                    .into_int_value();
                let pass = self.context.append_basic_block(function, "guard.pass");
                self.branch_on(holds, pass, retry);
                self.builder.position_at_end(pass);
            }
            self.commit(temp, counted, &arm.bindings);
            let value = self.block(&arm.body, None);
            if !self.terminated() {
                // A `unit` `match` discards its arms' values, as a `unit` `if` does.
                if ty == Type::UNIT {
                    self.discard(value, arm.body.ty);
                }
                self.release_to(outer);
                ends.push((value, self.current_block()));
            }
            self.builder.position_at_end(fail);
        }
        self.live.truncate(outer);
        let trap = Intrinsic::find("llvm.trap")
            .and_then(|trap| trap.get_declaration(&self.module, &[]))
            .expect("LLVM should provide `llvm.trap`, which takes no type parameters");
        self.call(trap, &[]);
        self.builder.build_unreachable().expect(POSITIONED);
        if ends.is_empty() {
            return None;
        }
        let merge = self.context.append_basic_block(function, "match.end");
        for (_, end) in &ends {
            self.builder.position_at_end(*end);
            self.branch_to(merge);
        }
        self.builder.position_at_end(merge);
        let llvm_ty = self.llvm_type(ty)?;
        let incoming: Vec<_> = ends
            .into_iter()
            .map(|(value, end)| (value.expect("the checker should give every arm that falls through a value of the `match`'s type"), end))
            .collect();
        Some(self.phi(llvm_ty, &incoming))
    }

    /// Gives `local` an `alloca` of its own, named `name`, in place of any slot `define` gave
    /// it, so that a value stored on one path is there on every path after it, without a `phi`.
    fn own_slot(&mut self, local: LocalId, name: &str) -> PointerValue<'ctx> {
        let ty = self
            .llvm_type(self.types[local.0])
            .expect("the checker should give a scrutinee and each binding a value type");
        let slot = self.entry_alloca(ty, name);
        self.locals[local.0] = Some(Slot::Alloca(slot, ty));
        slot
    }

    /// Stores `value` into the slot `own_slot` gave `local`, without a retain.
    fn set_binding(&self, local: LocalId, value: BasicValueEnum<'ctx>) {
        let Some(Slot::Alloca(slot, _)) = self.locals[local.0] else {
            unreachable!("`match_expr` should give each binding an `alloca` before its test")
        };
        self.store(slot, value);
    }

    /// The address of the value `at` names.
    fn address(&self, at: At<'ctx>) -> PointerValue<'ctx> {
        match at {
            At::Root(slot) => slot,
            At::Field(slot) => self
                .builder
                .build_load(
                    self.context.ptr_type(AddressSpace::default()),
                    slot,
                    "place",
                )
                .expect(POSITIONED)
                .into_pointer_value(),
        }
    }

    /// The value of type `ty` at `at`, borrowed.
    fn load_at(&self, at: At<'ctx>, ty: Type) -> BasicValueEnum<'ctx> {
        let llvm_ty = self
            .llvm_type(ty)
            .expect("the checker should test and bind only values of a value type");
        self.builder
            .build_load(llvm_ty, self.address(at), "value")
            .expect(POSITIONED)
    }

    /// Tests `pattern` against the value of type `ty` at `at`: branches to `fail` when it does
    /// not match, and leaves the builder where it does, with each binding's slot set. Returns
    /// the block that a test after this one branches to when it fails, its retry target: `fail`
    /// for a pattern with no `|`, the retry target of its last sub-test for a variant, and for
    /// an `|` a block that resumes the alternative that matched.
    fn test(
        &mut self,
        pattern: &Pattern,
        at: At<'ctx>,
        ty: Type,
        fail: BasicBlock<'ctx>,
    ) -> BasicBlock<'ctx> {
        match &pattern.kind {
            PatternKind::Wild => fail,
            PatternKind::Bind(local) => {
                let value = self.load_at(at, ty);
                self.set_binding(*local, value);
                fail
            }
            PatternKind::Variant { variant, fields } => {
                let TypeKind::Enum(id) = ty.kind() else {
                    unreachable!("the checker should allow a variant pattern only at an enum")
                };
                let tag = self.load_tag(id, self.address(at));
                let wanted =
                    u64::try_from(*variant).expect("a variant index should fit in a `u64`");
                let wanted = self.enum_layout(id).tag.const_int(wanted, false);
                let matched = self.int_compare(IntPredicate::EQ, tag, wanted);
                let next = self
                    .context
                    .append_basic_block(self.current_function(), "variant");
                self.branch_on(matched, next, fail);
                self.builder.position_at_end(next);
                let program = self.program;
                let payloads = &program.enums[id.0].variants[*variant].fields;
                let mut retry = fail;
                for (field, (sub, payload)) in fields.iter().zip(payloads).enumerate() {
                    match &sub.kind {
                        PatternKind::Wild => {}
                        PatternKind::Bind(local) => {
                            let value = self.payload_value(self.address(at), id, *variant, field);
                            self.set_binding(*local, value);
                        }
                        PatternKind::Variant { .. }
                        | PatternKind::Bool(_)
                        | PatternKind::Range { .. }
                        | PatternKind::Str(_)
                        | PatternKind::Or(_) => {
                            let place = self.payload_place(self.address(at), id, *variant, field);
                            let slot = self.entry_alloca(place.get_type().into(), "place");
                            self.store(slot, place.into());
                            retry = self.test(sub, At::Field(slot), payload.ty, retry);
                        }
                    }
                }
                retry
            }
            PatternKind::Bool(_) | PatternKind::Range { .. } | PatternKind::Str(_) => {
                let value = self.load_at(at, ty);
                let matched = self.literal_matches(&pattern.kind, value, ty);
                let next = self
                    .context
                    .append_basic_block(self.current_function(), "literal");
                self.branch_on(matched, next, fail);
                self.builder.position_at_end(next);
                fail
            }
            PatternKind::Or(alternatives) => self.alternatives(alternatives, at, ty, fail),
        }
    }

    /// Whether `value`, of type `ty`, is the literal or lies in the range `kind`. A range
    /// compares signed on `i64` and unsigned on `u8` and `char`; a string compares with the
    /// literal's global, borrowed, so the test retains and releases nothing.
    fn literal_matches(
        &self,
        kind: &PatternKind,
        value: BasicValueEnum<'ctx>,
        ty: Type,
    ) -> IntValue<'ctx> {
        match kind {
            PatternKind::Bool(b) => self.int_compare(
                IntPredicate::EQ,
                value.into_int_value(),
                self.context.bool_type().const_int(u64::from(*b), false),
            ),
            PatternKind::Range { lo, hi } => {
                let value = value.into_int_value();
                let bound = |n: i64| value.get_type().const_int(n.cast_unsigned(), false);
                if lo == hi {
                    return self.int_compare(IntPredicate::EQ, value, bound(*lo));
                }
                let (ge, le) = if ty == Type::I64 {
                    (IntPredicate::SGE, IntPredicate::SLE)
                } else {
                    (IntPredicate::UGE, IntPredicate::ULE)
                };
                let above = self.int_compare(ge, value, bound(*lo));
                let below = self.int_compare(le, value, bound(*hi));
                self.builder
                    .build_and(above, below, "in_range")
                    .expect(POSITIONED)
            }
            PatternKind::Str(text) => {
                self.equal(Type::STRING, value, self.string_global(text).into())
            }
            PatternKind::Wild
            | PatternKind::Bind(_)
            | PatternKind::Variant { .. }
            | PatternKind::Or(_) => {
                unreachable!("`test` should pass only a literal or a range pattern")
            }
        }
    }

    /// `a1 | … | am`. An alloca `tried` holds the index of the alternative being tried, 0 on
    /// entry. `dispatch` switches on `tried` to that alternative's test, whose failure target
    /// stores the next index and branches back to `dispatch`; past the last, `dispatch` branches
    /// to `fail`. Every alternative's success joins in one block, where the builder is left. The
    /// retry target switches on `tried` to the retry target of the alternative that matched, so
    /// a later failure resumes its innermost choice point first. Each block loads the addresses
    /// it tests from their slots, so no block uses a value defined in a block that `dispatch` or
    /// the retry switch bypasses; bindings are stored by whichever alternative matched, so the
    /// join needs no `phi`.
    fn alternatives(
        &mut self,
        alternatives: &[Pattern],
        at: At<'ctx>,
        ty: Type,
        fail: BasicBlock<'ctx>,
    ) -> BasicBlock<'ctx> {
        let function = self.current_function();
        let index = self.context.i32_type();
        let tried = self.entry_alloca(index.into(), "alternative");
        self.store(tried, index.const_zero().into());
        let dispatch = self.context.append_basic_block(function, "or.dispatch");
        let join = self.context.append_basic_block(function, "or.join");
        self.branch_to(dispatch);
        let mut starts = Vec::with_capacity(alternatives.len());
        let mut retries = Vec::with_capacity(alternatives.len());
        for (i, alternative) in (0..).zip(alternatives) {
            let start = self.context.append_basic_block(function, "or.try");
            let next = self.context.append_basic_block(function, "or.next");
            self.builder.position_at_end(next);
            self.store(tried, index.const_int(i + 1, false).into());
            self.branch_to(dispatch);
            self.builder.position_at_end(start);
            let retry = self.test(alternative, at, ty, next);
            self.branch_to(join);
            starts.push((index.const_int(i, false), start));
            retries.push((index.const_int(i, false), retry));
        }
        let retry = self.context.append_basic_block(function, "or.retry");
        for (block, cases) in [(dispatch, &starts), (retry, &retries)] {
            self.builder.position_at_end(block);
            let current = self
                .builder
                .build_load(index, tried, "tried")
                .expect(POSITIONED)
                .into_int_value();
            self.builder
                .build_switch(current, fail, cases)
                .expect(POSITIONED);
        }
        self.builder.position_at_end(join);
        retry
    }

    /// The commit of an arm whose pattern matched: each counted binding adds 1, then `temp`,
    /// when `counted`, is released and leaves `live`, and the counted bindings join it.
    fn commit(&mut self, temp: LocalId, counted: bool, bindings: &[LocalId]) {
        let enums = &self.program.enums;
        let held: Vec<_> = bindings
            .iter()
            .copied()
            .filter(|local| self.types[local.0].is_counted(enums))
            .collect();
        for &local in &held {
            let value = self
                .load_local(local)
                .expect("a binding should have its value once its arm's pattern matched");
            self.retain(value, self.types[local.0]);
        }
        if counted {
            let value = self
                .load_local(temp)
                .expect("the scrutinee should be in its slot until the commit");
            self.release(value, self.types[temp.0]);
            self.live.pop();
        }
        self.live.extend(held);
    }
}
