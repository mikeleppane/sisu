//! Standard input: `read_line`, through the runtime's `sisu_read_line`.

use inkwell::AddressSpace;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::values::{FunctionValue, PointerValue};

use super::{Codegen, POSITIONED};

impl<'ctx> Codegen<'ctx, '_> {
    /// `sisu_read_line(len) -> ptr` from the runtime, declared on first use. It never unwinds,
    /// and has no `memory` attribute: each call overwrites the line an earlier call returned,
    /// which generated code can reach, so no encoding of its effects is sound.
    fn read_line_function(&self) -> FunctionValue<'ctx> {
        self.module
            .get_function("sisu_read_line")
            .unwrap_or_else(|| {
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let function = self.module.add_function(
                    "sisu_read_line",
                    ptr_type.fn_type(&[ptr_type.into()], false),
                    None,
                );
                function.add_attribute(
                    AttributeLoc::Function,
                    self.context
                        .create_enum_attribute(Attribute::get_named_enum_kind_id("nounwind"), 0),
                );
                function
            })
    }

    /// `ReadLine`: the next line of standard input as a new string, copied at once, before
    /// anything can call `sisu_read_line` again; `null`, which is `None`, at the end of input.
    pub(super) fn read_line(&mut self) -> PointerValue<'ctx> {
        let i64_type = self.context.i64_type();
        let len_slot = self.entry_alloca(i64_type.into(), "len");
        let bytes = self
            .builder
            .build_call(self.read_line_function(), &[len_slot.into()], "line")
            .expect(POSITIONED)
            .try_as_basic_value()
            .basic()
            .expect("`sisu_read_line` should return a pointer, as the runtime declares it")
            .into_pointer_value();
        let function = self.current_function();
        let (copy, join) = (
            self.context.append_basic_block(function, "copy"),
            self.context.append_basic_block(function, "join"),
        );
        let ended = self.current_block();
        let none = self
            .builder
            .build_is_null(bytes, "ended")
            .expect(POSITIONED);
        self.builder
            .build_conditional_branch(none, join, copy)
            .expect(POSITIONED);
        self.builder.position_at_end(copy);
        let len = self
            .builder
            .build_load(i64_type, len_slot, "len")
            .expect(POSITIONED)
            .into_int_value();
        let string = self.string_from_bytes(bytes, len);
        self.branch_to(join);
        self.builder.position_at_end(join);
        let null = self.context.ptr_type(AddressSpace::default()).const_null();
        self.phi(null.get_type(), &[(null, ended), (string, copy)])
            .into_pointer_value()
    }
}
