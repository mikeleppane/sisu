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

use diagnostic::{Diagnostic, Severity};
use inkwell::context::Context;

const USAGE: &str =
    "usage: sisuc <input.sisu> <output> | --emit tokens|ast|ir <input.sisu> | --check <input.sisu>";

/// What the command line asks for.
enum Mode {
    Emit { stage: Stage, input: PathBuf },
    Check { input: PathBuf },
    // The fixed program, until stage 4 replaces it with the real compile path.
    Hello { output: PathBuf },
}

enum Stage {
    Tokens,
    Ast,
}

fn parse_args(args: &[OsString]) -> Result<Mode, String> {
    match args {
        [flag, stage, input] if flag == "--emit" => {
            let stage = match stage.to_str() {
                Some("tokens") => Stage::Tokens,
                Some("ast") => Stage::Ast,
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
        [output] if !output.to_string_lossy().starts_with('-') => Ok(Mode::Hello {
            output: PathBuf::from(output),
        }),
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
        Mode::Hello { output } => match compile(&output) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("sisuc: {message}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Prints one stage's output for `input` to stdout.
fn emit(stage: &Stage, input: &Path) -> ExitCode {
    let path = input.to_string_lossy();
    let source = match fs::read_to_string(input) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("sisuc: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
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
    }
}

/// Lexes, parses and checks `input`; prints every diagnostic. Fails if any is an error.
fn run_check(input: &Path) -> ExitCode {
    let path = input.to_string_lossy();
    let source = match fs::read_to_string(input) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("sisuc: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let diagnostics = match lexer::lex(&source).and_then(|tokens| parser::parse(&tokens)) {
        Ok(program) => check::check(&program),
        Err(d) => vec![d],
    };
    report(&path, &source, &diagnostics);
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Prints each diagnostic to stderr, followed by a blank line.
fn report(path: &str, source: &str, diagnostics: &[Diagnostic]) {
    for d in diagnostics {
        eprintln!("{}\n", d.render(path, source));
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
