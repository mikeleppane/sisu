//! End to end: `sisuc` compiles each program in `tests/programs`, and the program
//! prints the right output or panics with the right message.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const PROGRAMS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/programs");
const MIN: &str = "    let min = -9_223_372_036_854_775_807 - 1";
const MAX: &str = "    let big = 9_223_372_036_854_775_807";

/// Compiles `<dir>/<name>.sisu`, run from `dir`, into `<CARGO_TARGET_TMPDIR>/<name>`.
fn compile(dir: &Path, name: &str) -> PathBuf {
    // `cargo nextest run` does not build the runtime archive that `sisuc` links
    // against, so build it here. This is a no-op when it is up to date.
    let built = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "-p", "sisu-runtime"])
        .status()
        .expect("cargo starts");
    assert!(built.success(), "building sisu-runtime failed");

    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .arg(format!("{name}.sisu"))
        .arg(&exe)
        .output()
        .expect("sisuc starts");
    assert!(
        out.status.success(),
        "sisuc failed on {name}.sisu:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    exe
}

/// Compiles and runs `tests/programs/<name>.sisu`.
fn run(name: &str) -> Output {
    let exe = compile(Path::new(PROGRAMS), name);
    Command::new(exe).output().expect("the program starts")
}

/// Runs `<name>.sisu` and checks that it exits 0, prints `<name>.out` and nothing on stderr.
fn assert_prints_out_file(name: &str) {
    let expected =
        fs::read_to_string(Path::new(PROGRAMS).join(format!("{name}.out"))).expect("reads .out");
    let out = run(name);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}

/// Checks that `out` is a panic: exit 101, `stdout`, and exactly the panic line on stderr.
fn assert_panics(out: &Output, stdout: &str, stderr: &str) {
    assert_eq!(out.status.code(), Some(101));
    assert_eq!(String::from_utf8_lossy(&out.stdout), stdout);
    assert_eq!(String::from_utf8_lossy(&out.stderr), stderr);
}

#[test]
fn hello() {
    assert_prints_out_file("hello");
}

#[test]
fn fib() {
    assert_prints_out_file("fib");
}

#[test]
fn semantics() {
    assert_prints_out_file("semantics");
}

#[test]
fn primes() {
    assert_prints_out_file("primes");
}

#[test]
fn loops() {
    assert_prints_out_file("loops");
}

#[test]
fn overflow() {
    assert_panics(
        &run("overflow"),
        "1\n",
        "sisu: panic at overflow.sisu:4:11: integer overflow\n",
    );
}

#[test]
fn div_zero() {
    assert_panics(
        &run("div_zero"),
        "",
        "sisu: panic at div_zero.sisu:2:5: division by zero\n",
    );
}

#[test]
fn div_overflow() {
    assert_panics(
        &run("div_overflow"),
        "",
        "sisu: panic at div_overflow.sisu:3:13: integer overflow\n",
    );
}

/// Writes `source` to `<CARGO_TARGET_TMPDIR>/<name>.sisu` and compiles it from there.
fn compile_tmp(name: &str, source: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join(format!("{name}.sisu")), source).expect("writes");
    compile(dir, name)
}

#[test]
fn panic_cases() {
    let cases = [
        (
            "sub_overflow",
            format!("fn main() {{\n{MIN}\n    print(min - 1)\n}}\n"),
            "3:11: integer overflow",
        ),
        (
            "mul_overflow",
            format!("fn main() {{\n{MAX}\n    print(big * 2)\n}}\n"),
            "3:11: integer overflow",
        ),
        (
            "neg_overflow",
            format!("fn main() {{\n{MIN}\n    print(-min)\n}}\n"),
            "3:11: integer overflow",
        ),
        (
            "rem_zero",
            "fn main() {\n    let z = 0\n    print(7 % z)\n}\n".to_string(),
            "3:11: division by zero",
        ),
        (
            "rem_overflow",
            format!("fn main() {{\n{MIN}\n    print(min % -1)\n}}\n"),
            "3:11: integer overflow",
        ),
        (
            "compound_overflow",
            "fn main() {\n    var x = 9_223_372_036_854_775_807\n    x += 1\n    print(x)\n}\n"
                .to_string(),
            "3:5: integer overflow",
        ),
    ];
    for (name, source, position_and_message) in cases {
        let out = Command::new(compile_tmp(name, &source))
            .output()
            .expect("the program starts");
        assert_panics(
            &out,
            "",
            &format!("sisu: panic at {name}.sisu:{position_and_message}\n"),
        );
    }
}

#[test]
fn panic_exits_101_when_stderr_write_fails() {
    // Writes to /dev/full fail with ENOSPC, so the runtime's panic line cannot be written.
    let full = File::options()
        .write(true)
        .open("/dev/full")
        .expect("Linux has /dev/full, the only target Sisu supports");
    let exe = compile_tmp(
        "stderr_full",
        "fn main() {\n    let z = 0\n    print(1 / z)\n}\n",
    );
    let status = Command::new(exe)
        .stderr(Stdio::from(full))
        .status()
        .expect("the program starts");
    assert_eq!(status.code(), Some(101));
}
