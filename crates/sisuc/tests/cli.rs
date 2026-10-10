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
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .starts_with("error: expected `fn`, found `let`\n --> parse_error_exits_1.sisu:1:1")
    );
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
