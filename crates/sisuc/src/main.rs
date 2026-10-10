//! `sisuc`, the Sisu compiler.

mod ast;
mod check;
mod codegen;
mod diagnostic;
mod lexer;
mod link;
mod parser;
mod tir;

use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use diagnostic::Diagnostic;
use inkwell::context::Context;
use tir::Program;

const USAGE: &str = "usage: sisuc [-O2] <input.sisu> <output> | [-O2] --emit ir <input.sisu> | --emit tokens|ast|tir|ir-raw <input.sisu> | --check <input.sisu>";

/// What the command line asks for.
enum Mode {
    Emit {
        stage: Stage,
        input: PathBuf,
        o2: bool,
    },
    Check {
        input: PathBuf,
    },
    Build {
        input: PathBuf,
        output: PathBuf,
        o2: bool,
    },
}

enum Stage {
    Tokens,
    Ast,
    Tir,
    Ir,
    IrRaw,
}

fn parse_args(args: &[OsString]) -> Result<Mode, String> {
    // `-O2` is accepted only as the first argument.
    let (o2, args) = match args {
        [flag, rest @ ..] if flag == "-O2" => (true, rest),
        _ => (false, args),
    };
    let mode = match args {
        [flag, stage, input] if flag == "--emit" => {
            let stage = match stage.to_str() {
                Some("tokens") => Stage::Tokens,
                Some("ast") => Stage::Ast,
                Some("tir") => Stage::Tir,
                Some("ir") => Stage::Ir,
                Some("ir-raw") => Stage::IrRaw,
                _ => return Err(USAGE.to_string()),
            };
            Ok(Mode::Emit {
                stage,
                input: PathBuf::from(input),
                o2,
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
                o2,
            })
        }
        _ => Err(USAGE.to_string()),
    }?;
    // The other modes run no optimizer.
    if o2
        && !matches!(
            mode,
            Mode::Build { .. }
                | Mode::Emit {
                    stage: Stage::Ir,
                    ..
                }
        )
    {
        return Err(USAGE.to_string());
    }
    Ok(mode)
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
        Mode::Emit { stage, input, o2 } => emit(&stage, &input, o2),
        Mode::Check { input } => run_check(&input),
        Mode::Build { input, output, o2 } => build(&input, &output, o2),
    }
}

/// Reads `input`, or prints why it cannot.
fn read(input: &Path) -> Option<String> {
    fs::read_to_string(input)
        .inspect_err(|e| eprintln!("sisuc: cannot read {}: {e}", input.to_string_lossy()))
        .ok()
}

/// Prints one stage's output for `input` to stdout.
fn emit(stage: &Stage, input: &Path, o2: bool) -> ExitCode {
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
        Stage::Tir => match front_end(&path, &source) {
            Some(program) => {
                println!("{program}");
                ExitCode::SUCCESS
            }
            None => ExitCode::FAILURE,
        },
        Stage::Ir => emit_ir(&path, &source, o2),
        Stage::IrRaw => emit_ir_raw(&path, &source),
    }
}

/// Prints the module for `source` before and after the pipeline, once it passes the checker.
fn emit_ir(path: &str, source: &str, o2: bool) -> ExitCode {
    let Some(program) = front_end(path, source) else {
        return ExitCode::FAILURE;
    };
    let context = Context::create();
    let module = codegen::compile(&context, &program, path, source);
    // `LLVMString`'s `Display` quotes and escapes the text; `to_string` does not.
    let before = module.print_to_string().to_string();
    let pipeline = codegen::pipeline(o2);
    match codegen::target_machine(o2).and_then(|machine| codegen::optimize(&module, &machine, o2)) {
        Ok(()) => {
            print!(
                "; before {pipeline}\n{before}\n; after {pipeline}\n{}",
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

/// Prints the verified module for `source`, as codegen made it.
fn emit_ir_raw(path: &str, source: &str) -> ExitCode {
    let Some(program) = front_end(path, source) else {
        return ExitCode::FAILURE;
    };
    let context = Context::create();
    let module = codegen::compile(&context, &program, path, source);
    match module.verify() {
        Ok(()) => {
            print!("{}", module.print_to_string().to_string());
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
fn build(input: &Path, output: &Path, o2: bool) -> ExitCode {
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
    match compile(&program, &path, &source, output, o2) {
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
    let (program, diagnostics) = check::check(&program);
    report(path, source, &diagnostics);
    program
}

/// Prints each diagnostic to stderr, followed by a blank line.
fn report(path: &str, source: &str, diagnostics: &[Diagnostic]) {
    for d in diagnostics {
        eprintln!("{}\n", d.render(path, source));
    }
}

/// Compiles `program` into the executable `output`, going through a temporary object file.
fn compile(
    program: &Program,
    path: &str,
    source: &str,
    output: &Path,
    o2: bool,
) -> Result<(), String> {
    let context = Context::create();
    let module = codegen::compile(&context, program, path, source);
    let machine = codegen::target_machine(o2)?;
    codegen::optimize(&module, &machine, o2)?;
    let code = codegen::object_code(&module, &machine)?;
    // A name of its own, so a user's `<output>.o` is left alone; the nanoseconds pick a
    // fresh name when one is taken.
    let pid = std::process::id();
    let names = (0..10).map(|_| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        format!("sisuc-{pid}-{nanos}.o")
    });
    let object = create_object(&std::env::temp_dir(), code.as_slice(), names)?;
    // Remove the object file whether or not the link worked; a link error wins.
    let linked = link::link(&object, output);
    let removed =
        fs::remove_file(&object).map_err(|e| format!("cannot remove {}: {e}", object.display()));
    linked.and(removed)
}

/// Writes `bytes` to a new file in `dir`, under the first of `names` that is free, and
/// returns its path.
fn create_object(
    dir: &Path,
    bytes: &[u8],
    names: impl IntoIterator<Item = String>,
) -> Result<PathBuf, String> {
    // `create_new` is `O_CREAT | O_EXCL`: it fails on any existing path, a symlink included,
    // so a name planted in a shared temp dir is never followed or overwritten.
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Owner-only from creation: other users of the shared temp dir cannot read the object.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    for name in names {
        let path = dir.join(name);
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot create {}: {e}", path.display())),
        };
        return match file.write_all(bytes) {
            Ok(()) => Ok(path),
            Err(e) => {
                let message = format!("cannot write {}: {e}", path.display());
                fs::remove_file(&path)
                    .map_err(|e| format!("{message}; cannot remove it: {e}"))
                    .and(Err(message))
            }
        };
    }
    Err(format!(
        "cannot create a temporary object file in {}",
        dir.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn object_file_never_follows_an_existing_path() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!("sisuc-create-object-{}", std::process::id()));
        // It is absent unless an earlier run failed, so a failure here is expected and harmless.
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).expect("creates the directory");
        let target = dir.join("target");
        fs::write(&target, "not sisuc's").expect("writes");
        symlink(&target, dir.join("taken.o")).expect("links");
        symlink(dir.join("absent"), dir.join("dangling.o")).expect("links");

        let names = ["taken.o", "dangling.o", "free.o"].map(String::from);
        let object = create_object(&dir, b"object", names).expect("`free.o` is free");
        assert_eq!(object, dir.join("free.o"));
        assert_eq!(fs::read(&object).expect("reads"), b"object");
        assert_eq!(fs::read_to_string(&target).expect("reads"), "not sisuc's");
        assert!(!dir.join("absent").exists());
        // With every name taken, it gives up without writing.
        assert!(create_object(&dir, b"object", ["taken.o".to_string()]).is_err());
        assert_eq!(fs::read_to_string(&target).expect("reads"), "not sisuc's");
        fs::remove_dir_all(&dir).expect("removes the directory");
    }

    #[cfg(unix)]
    #[test]
    fn object_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("sisuc-object-mode-{}", std::process::id()));
        // It is absent unless an earlier run failed, so a failure here is expected and harmless.
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).expect("creates the directory");
        let object = create_object(&dir, b"object", ["private.o".to_string()]).expect("creates");
        let mode = fs::metadata(&object)
            .expect("reads metadata")
            .permissions()
            .mode();
        fs::remove_dir_all(&dir).expect("removes the directory");
        assert_eq!(mode & 0o077, 0, "mode {mode:o}");
    }
}
