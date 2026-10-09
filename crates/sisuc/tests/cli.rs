//! End to end: the `sisuc` command line.

use std::fs;
use std::path::Path;
use std::process::Command;

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
