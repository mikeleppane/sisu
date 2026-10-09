//! `sisuc`, the Sisu compiler.

mod ast;
mod check;
mod codegen;
mod diagnostic;
mod lexer;
mod link;
mod parser;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ast::Program;
use diagnostic::{Diagnostic, Severity};
use inkwell::context::Context;

const USAGE: &str =
    "usage: sisuc <input.sisu> <output> | --emit tokens|ast|ir <input.sisu> | --check <input.sisu>";

/// What the command line asks for.
enum Mode {
    Emit { stage: Stage, input: PathBuf },
    Check { input: PathBuf },
    Build { input: PathBuf, output: PathBuf },
}

enum Stage {
    Tokens,
    Ast,
    Ir,
}

fn parse_args(args: &[OsString]) -> Result<Mode, String> {
    match args {
        [flag, stage, input] if flag == "--emit" => {
            let stage = match stage.to_str() {
                Some("tokens") => Stage::Tokens,
                Some("ast") => Stage::Ast,
                Some("ir") => Stage::Ir,
                _ => return Err(USAGE.to_string()),
            };
            Ok(Mode::Emit {
                stage,
                input: PathBuf::from(input),
            })
        }
        [flag, input] if flag == "--check" => Ok(Mode::Check {
            input: PathBuf::from(input),
        }),
        [input, output]
            if [input, output]
                .iter()
                .all(|a| !a.to_string_lossy().starts_with('-')) =>
        {
            Ok(Mode::Build {
                input: PathBuf::from(input),
                output: PathBuf::from(output),
            })
        }
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mode = match parse_args(&args) {
        Ok(mode) => mode,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::FAILURE;
        }
    };
    match mode {
        Mode::Emit { stage, input } => emit(&stage, &input),
        Mode::Check { input } => run_check(&input),
        Mode::Build { input, output } => build(&input, &output),
    }
}

/// Reads `input`, or prints why it cannot.
fn read(input: &Path) -> Option<String> {
    fs::read_to_string(input)
        .inspect_err(|e| eprintln!("sisuc: cannot read {}: {e}", input.to_string_lossy()))
        .ok()
}

/// Prints one stage's output for `input` to stdout.
fn emit(stage: &Stage, input: &Path) -> ExitCode {
    let path = input.to_string_lossy();
    let Some(source) = read(input) else {
        return ExitCode::FAILURE;
    };
    let tokens = match lexer::lex(&source) {
        Ok(tokens) => tokens,
        Err(d) => {
            report(&path, &source, &[d]);
            return ExitCode::FAILURE;
        }
    };
    match stage {
        Stage::Tokens => {
            print!("{}", lexer::dump(&source, &tokens));
            ExitCode::SUCCESS
        }
        Stage::Ast => match parser::parse(&tokens) {
            Ok(program) => {
                println!("{program}");
                ExitCode::SUCCESS
            }
            Err(d) => {
                report(&path, &source, &[d]);
                ExitCode::FAILURE
            }
        },
        Stage::Ir => emit_ir(&path, &source),
    }
}

/// Prints the module for `source` before and after `mem2reg`, once it passes the checker.
fn emit_ir(path: &str, source: &str) -> ExitCode {
    let Some(program) = front_end(path, source) else {
        return ExitCode::FAILURE;
    };
    let context = Context::create();
    let module = codegen::compile(&context, &program, path, source);
    // `LLVMString`'s `Display` quotes and escapes the text; `to_string` does not.
    let before = module.print_to_string().to_string();
    match codegen::target_machine().and_then(|machine| codegen::run_mem2reg(&module, &machine)) {
        Ok(()) => {
            print!(
                "; before mem2reg\n{before}\n; after mem2reg\n{}",
                module.print_to_string().to_string()
            );
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("sisuc: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Lexes, parses and checks `input`; prints every diagnostic. Fails if any is an error.
fn run_check(input: &Path) -> ExitCode {
    match read(input).and_then(|source| front_end(&input.to_string_lossy(), &source)) {
        Some(_) => ExitCode::SUCCESS,
        None => ExitCode::FAILURE,
    }
}

/// Compiles `input` into the executable `output`.
fn build(input: &Path, output: &Path) -> ExitCode {
    // `cc -o` would replace the source with the executable.
    let overwrites_input = match (fs::canonicalize(input), fs::canonicalize(output)) {
        (Ok(input), Ok(output)) => input == output,
        _ => input == output,
    };
    if overwrites_input {
        eprintln!(
            "sisuc: output {} would overwrite the input",
            output.to_string_lossy()
        );
        return ExitCode::FAILURE;
    }
    let path = input.to_string_lossy();
    let Some(source) = read(input) else {
        return ExitCode::FAILURE;
    };
    let Some(program) = front_end(&path, &source) else {
        return ExitCode::FAILURE;
    };
    match compile(&program, &path, &source, output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("sisuc: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Lexes, parses and checks `source`; prints every diagnostic. Returns the program unless
/// a diagnostic is an error.
fn front_end(path: &str, source: &str) -> Option<Program> {
    let program = match lexer::lex(source).and_then(|tokens| parser::parse(&tokens)) {
        Ok(program) => program,
        Err(d) => {
            report(path, source, &[d]);
            return None;
        }
    };
    let diagnostics = check::check(&program);
    report(path, source, &diagnostics);
    diagnostics
        .iter()
        .all(|d| d.severity != Severity::Error)
        .then_some(program)
}

/// Prints each diagnostic to stderr, followed by a blank line.
fn report(path: &str, source: &str, diagnostics: &[Diagnostic]) {
    for d in diagnostics {
        eprintln!("{}\n", d.render(path, source));
    }
}

/// Compiles `program` into the executable `output`, going through a temporary object file.
fn compile(program: &Program, path: &str, source: &str, output: &Path) -> Result<(), String> {
    let context = Context::create();
    let module = codegen::compile(&context, program, path, source);
    // A name of its own, so a user's `<output>.o` is left alone.
    let object = std::env::temp_dir().join(format!("sisuc-{}.o", std::process::id()));
    let machine = codegen::target_machine()?;
    codegen::run_mem2reg(&module, &machine)?;
    codegen::write_object(&module, &machine, &object)?;
    // Remove the object file whether or not the link worked; a link error wins.
    let linked = link::link(&object, output);
    let removed =
        fs::remove_file(&object).map_err(|e| format!("cannot remove {}: {e}", object.display()));
    linked.and(removed)
}
