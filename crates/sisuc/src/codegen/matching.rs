//! `match`. The scrutinee is evaluated once into `temp`, a local the `match` owns. The arms'
//! patterns are tested in order, in place against `temp`; a binding takes a borrowed copy of its
//! bits while the patterns run. The first arm whose pattern matches commits: each binding adds 1,
//! `temp` is released, and the body runs with the bindings live. The arms' values join in a
//! `phi`. After the last arm the `match` traps: the checker proves that some arm matches, and a
//! checker bug must not become undefined behaviour.
//!
//! shortcut: bindings add 1 and the match then releases its scrutinee, so a part it held alone is
//! counted up and down once; move fields out when an enum benchmark shows that counting or the
//! box's free and allocation matter; box reuse depends on it (roadmap, "Reusing a box in match")

use inkwell::basic_block::BasicBlock;
use inkwell::intrinsics::Intrinsic;
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use super::{Codegen, POSITIONED, Slot};
use crate::tir::{Arm, Expr, LocalId, Pattern, PatternKind, Type};

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
        ty: &Type,
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
            assert!(arm.guard.is_none(), "guards in codegen land in Task 12");
            self.live.truncate(outer);
            if counted {
                self.live.push(temp);
            }
            for &local in &arm.bindings {
                self.own_slot(local, "bind");
            }
            // The last arm's failure block is the trap.
            let fail = self.context.append_basic_block(function, "arm.next");
            self.test(&arm.pattern, At::Root(root), &scrutinee.ty, fail);
            self.commit(temp, counted, &arm.bindings);
            let value = self.block(&arm.body, None);
            if !self.terminated() {
                // A `unit` `match` discards its arms' values, as a `unit` `if` does.
                if *ty == Type::Unit {
                    self.discard(value, &arm.body.ty);
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
            .llvm_type(&self.types[local.0])
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

    /// Tests `pattern` against the value of type `ty` at `at`: branches to `fail` when it does
    /// not match, and leaves the builder where it does, with each binding's slot set. Returns
    /// the block that a test after this one branches to when it fails, its retry target: `fail`
    /// for `_` and a name, and the retry target of its last sub-test for a variant.
    fn test(
        &mut self,
        pattern: &Pattern,
        at: At<'ctx>,
        ty: &Type,
        fail: BasicBlock<'ctx>,
    ) -> BasicBlock<'ctx> {
        match &pattern.kind {
            PatternKind::Wild => fail,
            PatternKind::Bind(local) => {
                let llvm_ty = self
                    .llvm_type(ty)
                    .expect("the checker should give a binding a value type");
                let value = self
                    .builder
                    .build_load(llvm_ty, self.address(at), "value")
                    .expect(POSITIONED);
                self.set_binding(*local, value);
                fail
            }
            PatternKind::Variant { variant, fields } => {
                let Type::Enum(id) = *ty else {
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
                            retry = self.test(sub, At::Field(slot), &payload.ty, retry);
                        }
                    }
                }
                retry
            }
            PatternKind::Bool(_) | PatternKind::Range { .. } | PatternKind::Str(_) => {
                panic!("literal and range patterns in codegen land in Task 12")
            }
            PatternKind::Or(_) => panic!("alternatives in codegen land in Task 12"),
        }
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
            self.retain(value, &self.types[local.0]);
        }
        if counted {
            let value = self
                .load_local(temp)
                .expect("the scrutinee should be in its slot until the commit");
            self.release(value, &self.types[temp.0]);
            self.live.pop();
        }
        self.live.extend(held);
    }
}
