//! End to end: the `sisuc` command line.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

#[test]
fn emit_tokens() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join("emit_tokens.sisu"), "fn main() {}\n").expect("writes");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args(["--emit", "tokens", "emit_tokens.sisu"])
        .output()
        .expect("sisuc starts");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("1:1 Fn fn\n1:4 Ident main\n"));
}

#[test]
fn emit_tokens_names_the_class_and_optional_tokens() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(
        dir.join("emit_new_tokens.sisu"),
        "class None is self . ? ?. ??",
    )
    .expect("writes");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args(["--emit", "tokens", "emit_new_tokens.sisu"])
        .output()
        .expect("sisuc starts");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let names: Vec<&str> = stdout
        .lines()
        .map(|l| l.split(' ').nth(1).expect("name column"))
        .collect();
    assert_eq!(
        names[..8],
        [
            "Class",
            "NoneKw",
            "Is",
            "SelfKw",
            "Dot",
            "Question",
            "QuestionDot",
            "QuestionQuestion"
        ]
    );
}

#[test]
fn lexer_error_exits_1() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join("lexer_error_exits_1.sisu"), "a & b\n").expect("writes");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args(["--emit", "tokens", "lexer_error_exits_1.sisu"])
        .output()
        .expect("sisuc starts");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .starts_with("error: unexpected character `&`\n --> lexer_error_exits_1.sisu:1:3")
    );
}

#[test]
fn bare_flag_is_a_usage_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(env!("CARGO_TARGET_TMPDIR"))
        .arg("--check")
        .output()
        .expect("sisuc starts");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("usage: sisuc"));
}

#[test]
fn emit_ast() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(
        dir.join("emit_ast.sisu"),
        "fn main() {\n    print(1 + 2)\n}\n",
    )
    .expect("writes");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args(["--emit", "ast", "emit_ast.sisu"])
        .output()
        .expect("sisuc starts");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "(fn main () unit (block (call print (+ 1 2))))\n"
    );
}

#[test]
fn emit_ast_keeps_compound_assign() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(
        dir.join("emit_ast_compound.sisu"),
        "fn main() {\n    var x = 1\n    x += 2\n}\n",
    )
    .expect("writes");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args(["--emit", "ast", "emit_ast_compound.sisu"])
        .output()
        .expect("sisuc starts");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "(fn main () unit (block (var x 1) (+= x 2)))\n"
    );
}

#[test]
fn parse_error_exits_1() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join("parse_error_exits_1.sisu"), "let x = 1\n").expect("writes");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args(["--emit", "ast", "parse_error_exits_1.sisu"])
        .output()
        .expect("sisuc starts");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with(
        "error: expected `fn` or `class`, found `let`\n --> parse_error_exits_1.sisu:1:1"
    ));
}

/// Runs `sisuc <flags> <name>.sisu` on `source`, saved as `<name>.sisu` in the target tmp dir.
fn run_on(name: &str, source: &str, flags: &[&str]) -> Output {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let file = format!("{name}.sisu");
    fs::write(dir.join(&file), source).expect("writes");
    Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args(flags)
        .arg(&file)
        .output()
        .expect("sisuc starts")
}

/// Runs `sisuc --check` on `source`, saved as `<name>.sisu` in the target tmp dir.
fn check(name: &str, source: &str) -> Output {
    run_on(name, source, &["--check"])
}

#[test]
fn check_valid_file() {
    let out = check("check_valid_file", "fn main() {\n    print(1)\n}\n");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}

#[test]
fn check_warning_exits_0() {
    let out = check(
        "check_warning_exits_0",
        "fn main() {\n    var count = 0\n    print(count)\n}\n",
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("warning: "));
}

#[test]
fn check_error_exits_1() {
    let out = check(
        "check_error_exits_1",
        "fn main() {\n    let n = 0\n    n = 1\n}\n",
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error: cannot assign to `n`"));
}

#[test]
fn check_warning_and_error_exits_1() {
    let out = check(
        "check_warning_and_error_exits_1",
        "fn f() { var x = 0 }\nfn main() { print(missing) }\n",
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let warning = stderr.find("warning: ").expect("a warning");
    let error = stderr.find("error: ").expect("an error");
    assert!(warning < error, "{stderr}");
}

#[test]
fn unreadable_input_exits_1() {
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(env!("CARGO_TARGET_TMPDIR"))
        .args(["--check", "no_such_dir/unreadable_input_exits_1.sisu"])
        .output()
        .expect("sisuc starts");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .starts_with("sisuc: cannot read no_such_dir/unreadable_input_exits_1.sisu: ")
    );
}

#[test]
fn mem2reg_removes_every_alloca() {
    // `loops.sisu` declares `var sq` inside the loop body: its `alloca` must still sit in the
    // entry block, or `mem2reg` leaves it in place.
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/programs"))
        .args(["--emit", "ir", "loops.sisu"])
        .output()
        .expect("sisuc starts");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let (before, after) = ir_sections(&stdout);
    assert!(before.contains("alloca"), "{before}");
    assert!(!after.contains("alloca"), "{after}");
}

/// Splits `--emit ir` output into the modules before and after `mem2reg`, and checks that
/// each defines `main`.
fn ir_sections(stdout: &str) -> (&str, &str) {
    let before = stdout
        .strip_prefix("; before mem2reg\n")
        .unwrap_or_else(|| panic!("no `; before mem2reg` header in\n{stdout}"));
    let (before, after) = before
        .split_once("\n; after mem2reg\n")
        .unwrap_or_else(|| panic!("no `; after mem2reg` line in\n{stdout}"));
    for section in [before, after] {
        assert!(section.contains("define void @sisu.main()"), "{section}");
    }
    (before, after)
}

#[test]
fn emit_ir_error_prints_no_ir() {
    let out = run_on(
        "emit_ir_error_prints_no_ir",
        "fn main() {\n    let n = 0\n    n = 1\n}\n",
        &["--emit", "ir"],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error: cannot assign to `n`"));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

#[test]
fn emit_ir_warning_still_prints_ir() {
    let out = run_on(
        "emit_ir_warning_still_prints_ir",
        "fn main() {\n    var count = 0\n    print(count)\n}\n",
        &["--emit", "ir"],
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with("warning: `count` is never reassigned")
    );
    ir_sections(&String::from_utf8_lossy(&out.stdout));
}

#[test]
fn emit_tir() {
    let out = run_on(
        "emit_tir",
        "fn main() {\n    var i = 0\n    while i < 2 {\n        i = i + 1\n    }\n}\n",
        &["--emit", "tir"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "(fn main () unit (block (var i#0 0) (loop (block (if (< i#0 2) (block (= i#0 (+ i#0 1))) (block (break)))))))\n"
    );
}

#[test]
fn emit_tir_error_prints_no_tir() {
    let out = run_on(
        "emit_tir_error_prints_no_tir",
        "fn main() {\n    let n = 0\n    n = 1\n}\n",
        &["--emit", "tir"],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error: cannot assign to `n`"));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

const FIB: &str = "fn fib(n: i64) -> i64 {\n    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }\n}\n\nfn main() {\n    print(fib(30))\n}\n";

#[test]
fn o2_combinations_are_usage_errors() {
    let cases: [&[&str]; 12] = [
        &["-O2", "--check", "f.sisu"],
        &["-O2", "--emit", "tokens", "f.sisu"],
        &["-O2", "--emit", "ast", "f.sisu"],
        &["-O2", "--emit", "tir", "f.sisu"],
        &["-O2", "--emit", "ir-raw", "f.sisu"],
        &["f.sisu", "out", "-O2"],
        &["--emit", "ir", "-O2", "f.sisu"],
        &["--check", "-O2", "f.sisu"],
        &["-O2", "-O2", "f.sisu", "out"],
        &["--emit", "ir", "-O2"],
        &["--check", "-O2"],
        &["--emit", "ir-raw", "-O2"],
    ];
    for args in cases {
        let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
            .current_dir(env!("CARGO_TARGET_TMPDIR"))
            .args(args)
            .output()
            .expect("sisuc starts");
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).starts_with("usage: sisuc"),
            "{args:?}"
        );
    }
}

#[test]
fn emit_ir_raw_parses_back() {
    let out = run_on("emit_ir_raw_parses_back", FIB, &["--emit", "ir-raw"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let context = inkwell::context::Context::create();
    let module = context
        .create_module_from_ir(
            inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(&out.stdout, "raw"),
        )
        .unwrap_or_else(|e| panic!("{e}\n{}", String::from_utf8_lossy(&out.stdout)));
    module.verify().expect("the module verifies");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("; before"));
}

#[test]
fn emit_ir_raw_is_the_ir_before_mem2reg() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/programs");
    let emit = |stage| {
        let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
            .current_dir(dir)
            .args(["--emit", stage, "loops.sisu"])
            .output()
            .expect("sisuc starts");
        assert_eq!(out.status.code(), Some(0));
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let (ir, raw) = (emit("ir"), emit("ir-raw"));
    let (before, _) = ir_sections(&ir);
    assert_eq!(raw.trim_end(), before.trim_end());
}

const SQUARE: &str =
    "fn sq(n: i64) -> i64 {\n    n * n\n}\n\nfn main() {\n    print(sq(3) + 4)\n}\n";

#[test]
fn emit_ir_o2() {
    let out = run_on("emit_ir_o2", SQUARE, &["-O2", "--emit", "ir"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("; before default<O2>\n"), "{stdout}");
    let (before, after) = stdout
        .split_once("\n; after default<O2>\n")
        .unwrap_or_else(|| panic!("{stdout}"));
    // Only the O2 pipeline inlines and folds the call; mem2reg alone keeps it.
    assert!(before.contains("call i64 @sisu.sq(i64 3)"), "{before}");
    assert!(!after.contains("call i64 @sisu.sq"), "{after}");
    assert!(after.contains("@sisu_print_int(i64 13)"), "{after}");
}
