//! End to end: `sisuc` compiles each program in `tests/programs`, and the program
//! prints the right output or panics with the right message.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const PROGRAMS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/programs");
const MIN: &str = "    let min = -9_223_372_036_854_775_807 - 1";
const MAX: &str = "    let big = 9_223_372_036_854_775_807";

/// `sisuc <input> <output>`, run from `dir`, once the runtime archive is built.
fn sisuc(dir: &Path, input: &str, output: &Path) -> Command {
    // `cargo nextest run` does not build the runtime archive that `sisuc` links
    // against, so build it here. This is a no-op when it is up to date.
    let built = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "-p", "sisu-runtime"])
        .status()
        .expect("cargo starts");
    assert!(built.success(), "building sisu-runtime failed");

    let mut command = Command::new(env!("CARGO_BIN_EXE_sisuc"));
    command.current_dir(dir).arg(input).arg(output);
    command
}

/// Compiles `<dir>/<name>.sisu`, run from `dir`, into `<CARGO_TARGET_TMPDIR>/<name>`.
fn compile(dir: &Path, name: &str) -> PathBuf {
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let out = sisuc(dir, &format!("{name}.sisu"), &exe)
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

#[test]
fn output_naming_the_input_is_refused() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let source = "fn main() {\n    print(1)\n}\n";
    let input = dir.join("overwrite_input.sisu");
    fs::write(&input, source).expect("writes");
    for output in ["overwrite_input.sisu", "./overwrite_input.sisu"] {
        let out = sisuc(dir, "overwrite_input.sisu", Path::new(output))
            .output()
            .expect("sisuc starts");
        assert_eq!(out.status.code(), Some(1), "output {output}");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            format!("sisuc: output {output} would overwrite the input\n")
        );
        assert_eq!(fs::read_to_string(&input).expect("reads"), source);
    }
}

/// An empty directory under the target tmp dir, for `sisuc` to use as `TMPDIR`.
fn empty_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    // It is absent on the first run, so a failure here is expected and harmless.
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).expect("creates the directory");
    dir
}

fn assert_empty(dir: &Path) {
    let left: Vec<_> = fs::read_dir(dir).expect("reads").collect();
    assert!(left.is_empty(), "sisuc left {left:?}");
}

#[test]
fn existing_object_file_survives_a_build() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join("keep_object.sisu"), "fn main() {}\n").expect("writes");
    fs::write(dir.join("keep_object.o"), "not sisuc's").expect("writes");
    let tmp = empty_dir("keep_object_tmp");
    let out = sisuc(dir, "keep_object.sisu", &dir.join("keep_object"))
        .env("TMPDIR", &tmp)
        .output()
        .expect("sisuc starts");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.join("keep_object.o")).expect("`keep_object.o` is still there"),
        "not sisuc's"
    );
    assert_empty(&tmp);
}

#[test]
fn failed_link_leaves_no_object_file() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join("link_fails.sisu"), "fn main() {}\n").expect("writes");
    // `cc` cannot write an executable over a directory.
    let output = empty_dir("link_fails");
    let tmp = empty_dir("link_fails_tmp");
    let out = sisuc(dir, "link_fails.sisu", &output)
        .env("TMPDIR", &tmp)
        .output()
        .expect("sisuc starts");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("sisuc: cc failed: "), "{stderr}");
    assert_empty(&tmp);
    assert!(!dir.join("link_fails.o").exists());
}
