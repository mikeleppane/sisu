//! Standard input and the program's arguments: `read_line`, through the runtime's
//! `sisu_read_line`, and `main`'s `args`, through `sisu_args_len` and `sisu_arg`.

use inkwell::AddressSpace;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::values::{FunctionValue, IntValue, PointerValue};

use super::{Codegen, POSITIONED, add_nounwind_memory};
use crate::tir::Type;

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
                self.add_nounwind(function);
                function
            })
    }

    fn add_nounwind(&self, function: FunctionValue<'ctx>) {
        function.add_attribute(
            AttributeLoc::Function,
            self.context
                .create_enum_attribute(Attribute::get_named_enum_kind_id("nounwind"), 0),
        );
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

    /// The C `main`'s `argc` and `argv`, passed as `count` and `strings`, as `main`'s `args`: a
    /// new `[string]` of the arguments after the program name. `sisu_args_len(argc)` counts
    /// them, and reads no memory.
    /// `sisu_arg(argv, index, len)` gives each, which is copied at once, before the next call can
    /// overwrite it; like `sisu_read_line`, it has no `memory` attribute. Neither unwinds.
    pub(super) fn args(
        &self,
        count: IntValue<'ctx>,
        strings: PointerValue<'ctx>,
    ) -> PointerValue<'ctx> {
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let args_len = self.module.add_function(
            "sisu_args_len",
            i64_type.fn_type(&[self.context.i32_type().into()], false),
            None,
        );
        add_nounwind_memory(&self.module, args_len, 0);
        let arg = self.module.add_function(
            "sisu_arg",
            ptr_type.fn_type(&[ptr_type.into(), i64_type.into(), ptr_type.into()], false),
            None,
        );
        self.add_nounwind(arg);
        let len = self
            .builder
            .build_call(args_len, &[count.into()], "args.len")
            .expect(POSITIONED)
            .try_as_basic_value()
            .basic()
            .expect("`sisu_args_len` should return an `i64`, as the runtime declares it")
            .into_int_value();
        let array = self.alloc_array(&Type::String, len, None);
        let len_slot = self.entry_alloca(i64_type.into(), "len");
        self.count_loop(len, |index| {
            let bytes = self
                .builder
                .build_call(arg, &[strings.into(), index.into(), len_slot.into()], "arg")
                .expect(POSITIONED)
                .try_as_basic_value()
                .basic()
                .expect("`sisu_arg` should return a pointer, as the runtime declares it")
                .into_pointer_value();
            let n = self.load(i64_type.into(), len_slot).into_int_value();
            let string = self.string_from_bytes(bytes, n);
            self.store(self.element_ptr(&Type::String, array, index), string.into());
        });
        array
    }
}
