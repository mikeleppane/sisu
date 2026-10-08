//! Builds LLVM IR through inkwell and writes it out as a native object file.

use std::path::Path;

use inkwell::OptimizationLevel;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};

/// Builds the module for the fixed program `fn main() { print_int(42) }`.
/// The parser replaces this once it exists.
pub(crate) fn hello_module(context: &Context) -> Module<'_> {
    let module = context.create_module("main");
    let i64_type = context.i64_type();
    let i32_type = context.i32_type();

    // declare void @sisu_print_int(i64): the runtime defines it, the linker finds it.
    let print_int_type = context.void_type().fn_type(&[i64_type.into()], false);
    let print_int = module.add_function("sisu_print_int", print_int_type, None);

    // define i32 @main(): the C startup code calls main and expects an exit status.
    let main_fn = module.add_function("main", i32_type.fn_type(&[], false), None);
    let entry = context.append_basic_block(main_fn, "entry");
    let builder = context.create_builder();
    builder.position_at_end(entry);
    builder
        .build_call(print_int, &[i64_type.const_int(42, false).into()], "")
        .expect("builder is positioned and the argument types match");
    builder
        .build_return(Some(&i32_type.const_zero()))
        .expect("builder is positioned");
    module
}

/// Writes `module` to `path` as an object file for the machine `sisuc` runs on.
pub(crate) fn write_object(module: &Module<'_>, path: &Path) -> Result<(), String> {
    Target::initialize_native(&InitializationConfig::default())?;
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
    // PIC because Ubuntu's `cc` links position-independent executables by default.
    let machine = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::None,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or("LLVM cannot create a target machine for this host")?;
    module.set_triple(&triple);
    module.set_data_layout(&machine.get_target_data().get_data_layout());
    module.verify().map_err(|e| e.to_string())?;
    machine
        .write_to_file(module, FileType::Object, path)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_module_calls_print_int_and_returns_zero() {
        let context = Context::create();
        let module = hello_module(&context);
        module.verify().expect("the module is valid IR");
        let expected = r#"; ModuleID = 'main'
source_filename = "main"

declare void @sisu_print_int(i64)

define i32 @main() {
entry:
  call void @sisu_print_int(i64 42)
  ret i32 0
}
"#;
        assert_eq!(module.print_to_string().to_string(), expected);
    }
}
