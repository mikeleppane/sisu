//! `sisuc`, the Sisu compiler.

mod codegen;
mod link;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use inkwell::context::Context;

fn main() -> ExitCode {
    let Some(output) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: sisuc <output>");
        return ExitCode::FAILURE;
    };
    match compile(&output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("sisuc: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Compiles the program into the executable `output`, going through `output.o`.
fn compile(output: &Path) -> Result<(), String> {
    let context = Context::create();
    let module = codegen::hello_module(&context);
    let object = output.with_added_extension("o");
    codegen::write_object(&module, &object)?;
    link::link(&object, output)?;
    fs::remove_file(&object).map_err(|e| format!("cannot remove {}: {e}", object.display()))
}
