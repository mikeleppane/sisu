//! End to end: `sisuc` compiles each program in `tests/programs`, and the program
//! prints the right output or panics with the right message.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const PROGRAMS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/programs");
const MIN: &str = "    let min = -9_223_372_036_854_775_807 - 1";
const MAX: &str = "    let big = 9_223_372_036_854_775_807";

/// `sisuc [-O2] <input> <output>`, run from `dir`, once the runtime archive is built.
fn sisuc(dir: &Path, input: &str, output: &Path, o2: bool) -> Command {
    // `cargo nextest run` does not build the runtime archive that `sisuc` links
    // against, so build it here. This is a no-op when it is up to date.
    let built = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "-p", "sisu-runtime"])
        .status()
        .expect("cargo starts");
    assert!(built.success(), "building sisu-runtime failed");

    let mut command = Command::new(env!("CARGO_BIN_EXE_sisuc"));
    command.current_dir(dir);
    if o2 {
        command.arg("-O2");
    }
    command.arg(input).arg(output);
    command
}

/// Compiles `<dir>/<name>.sisu`, run from `dir`, into `<CARGO_TARGET_TMPDIR>/<name>`, or
/// `<name>-O2` with `o2`.
fn compile(dir: &Path, name: &str, o2: bool) -> PathBuf {
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(if o2 {
        format!("{name}-O2")
    } else {
        name.to_string()
    });
    let out = sisuc(dir, &format!("{name}.sisu"), &exe, o2)
        .output()
        .expect("sisuc starts");
    assert!(
        out.status.success(),
        "sisuc failed on {name}.sisu:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    exe
}

/// Compiles, at `-O2` if `o2`, and runs `tests/programs/<name>.sisu`.
fn run(name: &str, o2: bool) -> Output {
    let exe = compile(Path::new(PROGRAMS), name, o2);
    Command::new(exe).output().expect("the program starts")
}

/// Runs `exe` under Valgrind; any leak or memory error makes it exit 1.
fn valgrind(exe: &Path) -> Output {
    Command::new("valgrind")
        .args([
            "--leak-check=full",
            "--errors-for-leak-kinds=definite,indirect,possible",
            "--error-exitcode=1",
        ])
        .arg(exe)
        .output()
        .expect("valgrind is required: install it with `apt install valgrind`")
}

/// Runs `<name>.sisu` and checks that it exits 0, prints `<name>.out` and nothing on stderr,
/// then that it does the same under Valgrind, and that the `-O2` build prints the same under
/// Valgrind.
fn assert_prints_out_file(name: &str) {
    let expected =
        fs::read_to_string(Path::new(PROGRAMS).join(format!("{name}.out"))).expect("reads .out");
    let exe = compile(Path::new(PROGRAMS), name, false);
    let out = Command::new(&exe).output().expect("the program starts");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");

    let o2 = compile(Path::new(PROGRAMS), name, true);
    assert!(
        fs::read(&exe).expect("reads plain build") != fs::read(&o2).expect("reads -O2 build"),
        "{name}: -O2 build is identical to the plain build"
    );
    for exe in [exe, o2] {
        let checked = valgrind(&exe);
        let valgrind_stderr = String::from_utf8_lossy(&checked.stderr);
        assert_eq!(
            checked.status.code(),
            Some(0),
            "{}: {valgrind_stderr}",
            exe.display()
        );
        assert_eq!(
            String::from_utf8_lossy(&checked.stdout),
            expected,
            "{}: {valgrind_stderr}",
            exe.display()
        );
    }
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
fn break_continue() {
    assert_prints_out_file("break_continue");
}

#[test]
fn short_circuit() {
    assert_prints_out_file("short_circuit");
}

#[test]
fn loop_return() {
    assert_prints_out_file("loop_return");
}

#[test]
fn bool_var() {
    assert_prints_out_file("bool_var");
}

#[test]
fn objects() {
    assert_prints_out_file("objects");
}

#[test]
fn ownership() {
    assert_prints_out_file("ownership");
}

#[test]
fn field_order() {
    assert_prints_out_file("field_order");
}

/// Each panicking program with its expected stdout and panic line.
const PANICS: [(&str, &str, &str); 3] = [
    (
        "overflow",
        "1\n",
        "sisu: panic at overflow.sisu:4:11: integer overflow\n",
    ),
    (
        "div_zero",
        "",
        "sisu: panic at div_zero.sisu:2:5: division by zero\n",
    ),
    (
        "div_overflow",
        "",
        "sisu: panic at div_overflow.sisu:3:13: integer overflow\n",
    ),
];

#[test]
fn overflow() {
    let (name, stdout, stderr) = PANICS[0];
    assert_panics(&run(name, false), stdout, stderr);
}

#[test]
fn div_zero() {
    let (name, stdout, stderr) = PANICS[1];
    assert_panics(&run(name, false), stdout, stderr);
}

#[test]
fn div_overflow() {
    let (name, stdout, stderr) = PANICS[2];
    assert_panics(&run(name, false), stdout, stderr);
}

#[test]
fn panics_survive_o2() {
    for (name, stdout, stderr) in PANICS {
        assert_panics(&run(name, true), stdout, stderr);
    }
}

/// Writes `source` to `<CARGO_TARGET_TMPDIR>/<name>.sisu` and compiles it from there.
fn compile_tmp(name: &str, source: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join(format!("{name}.sisu")), source).expect("writes");
    compile(dir, name, false)
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
        let out = sisuc(dir, "overwrite_input.sisu", Path::new(output), false)
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
    let out = sisuc(dir, "keep_object.sisu", &dir.join("keep_object"), false)
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
    let out = sisuc(dir, "link_fails.sisu", &output, false)
        .env("TMPDIR", &tmp)
        .output()
        .expect("sisuc starts");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("sisuc: cc failed: "), "{stderr}");
    assert_empty(&tmp);
    assert!(!dir.join("link_fails.o").exists());
}

/// Runs `sisuc <name>.sisu <name>` on `source` in the target tmp dir, after removing any
/// earlier `<name>`; returns the output and the executable's path.
fn build_tmp(name: &str, source: &str) -> (Output, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join(format!("{name}.sisu")), source).expect("writes");
    let exe = dir.join(name);
    // It is absent on the first run, so a failure here is expected and harmless.
    let _ = fs::remove_file(&exe);
    let out = sisuc(dir, &format!("{name}.sisu"), &exe, false)
        .output()
        .expect("sisuc starts");
    (out, exe)
}

#[test]
fn build_error_makes_no_executable() {
    let (out, exe) = build_tmp(
        "build_error_makes_no_executable",
        "fn main() {\n    let n = 0\n    n = 1\n}\n",
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error: cannot assign to `n`"));
    assert!(!exe.exists());
}

#[test]
fn build_warning_still_makes_the_executable() {
    let (out, exe) = build_tmp(
        "build_warning_still_makes_the_executable",
        "fn main() {\n    var count = 0\n    print(count)\n}\n",
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with("warning: `count` is never reassigned")
    );
    let run = Command::new(exe).output().expect("the program starts");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "0\n");
}
