//! End to end: programs that need more than a `tests/run` or `tests/run-fail` case (see
//! `cases.rs`), such as a small stack, an allocation count or a generated source, and the bless
//! rules of the `cases` harness.

use std::fs::{self, File};
use std::os::unix::process::ExitStatusExt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;

mod support;

const PROGRAMS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/programs");
const RUN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/run");
const MIN: &str = "    let min = -9_223_372_036_854_775_807 - 1";
const MAX: &str = "    let big = 9_223_372_036_854_775_807";

/// Compiles `<dir>/<name>.sisu`, run from `dir`, into `<CARGO_TARGET_TMPDIR>/<name>`, or
/// `<name>-O2` with `o2`.
fn compile_to_tmp(dir: &Path, name: &str, o2: bool) -> PathBuf {
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(if o2 {
        format!("{name}-O2")
    } else {
        name.to_string()
    });
    support::compile(dir, name, &exe, if o2 { &["-O2"] } else { &[] });
    exe
}

/// Checks that `out` is a panic: exit 101, `stdout`, and exactly the panic line on stderr.
fn assert_panics(out: &Output, stdout: &str, stderr: &str) {
    assert_eq!(out.status.code(), Some(101));
    assert_eq!(String::from_utf8_lossy(&out.stdout), stdout);
    assert_eq!(String::from_utf8_lossy(&out.stderr), stderr);
}

/// The programs the small-stack tests run. They are not `tests/run` cases: a million objects
/// under Valgrind is too slow, and `deepchain` overflows on purpose.
const SMALL_STACK: [&str; 3] = ["longlist", "rightskew", "deepchain"];

/// `tests/programs` holds exactly the `SMALL_STACK` programs; every other program is a case in
/// `tests/run` or `tests/run-fail`.
#[test]
fn every_program_has_a_run() {
    let mut files: Vec<String> = fs::read_dir(PROGRAMS)
        .expect("tests/programs should be readable")
        .map(|entry| entry.expect("a directory entry should be readable").path())
        // Tools may leave a directory here, such as `.claude/`; only files are programs.
        .filter(|path| !path.is_dir())
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .expect("a file name should be UTF-8")
                .to_string()
        })
        .collect();
    files.sort();
    let mut expected = SMALL_STACK.map(|name| format!("{name}.sisu"));
    expected.sort();
    assert_eq!(files, expected);
}

/// The `N` of the `total heap usage: N allocs` line in Valgrind's stderr, thousands separators
/// removed; panics when the line is missing.
fn allocs_in(valgrind_stderr: &str) -> u64 {
    let (_, after) = valgrind_stderr
        .split_once("total heap usage: ")
        .unwrap_or_else(|| panic!("no `total heap usage` line in:\n{valgrind_stderr}"));
    let digits: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(|c| *c != ',')
        .collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("no allocation count after `total heap usage: ` in:\n{after}"))
}

/// Runs `exe` under Valgrind and returns its stdout and `allocs_in` of its stderr. Valgrind runs
/// without `-q`, which prints the heap summary.
fn heap_allocs(exe: &Path) -> (String, u64) {
    let out = support::valgrind(exe);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{}: {stderr}", exe.display());
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        allocs_in(&stderr),
    )
}

/// Builds `<name>.sisu` as written and with its first line `fn iterations() -> i64 { 1000 }`
/// changed to `{ 0 }`, at `-O2` if `o2`, and returns both builds' stdout and the difference in
/// allocations. The zero-iteration build is the baseline: the runtime's own allocations cancel.
fn extra_allocs(name: &str, o2: bool) -> (String, String, i64) {
    const FIRST_LINE: &str = "fn iterations() -> i64 { 1000 }\n";
    let source =
        fs::read_to_string(Path::new(RUN).join(format!("{name}.sisu"))).expect("reads the program");
    let rest = source
        .strip_prefix(FIRST_LINE)
        .unwrap_or_else(|| panic!("{name}.sisu should start with `{}`", FIRST_LINE.trim_end()));
    let baseline = format!("{}{rest}", FIRST_LINE.replace("1000", "0"));

    // Own file names, so these builds never share an executable with the test of `<name>.out`.
    let (stdout, allocs) = heap_allocs(&compile_tmp(&format!("{name}_k1000"), &source, o2));
    let (base_stdout, base_allocs) =
        heap_allocs(&compile_tmp(&format!("{name}_k0"), &baseline, o2));
    let extra = i64::try_from(allocs).expect("a count fits i64")
        - i64::try_from(base_allocs).expect("a count fits i64");
    (stdout, base_stdout, extra)
}

#[test]
fn allocs_in_reads_grouped_counts() {
    let stderr = "==1== total heap usage: 1,004 allocs, 1,004 frees, 9 bytes allocated";
    assert_eq!(allocs_in(stderr), 1004);
}

#[test]
#[should_panic(expected = "no `total heap usage` line")]
fn allocs_in_panics_without_the_summary_line() {
    allocs_in("==1== All heap blocks were freed -- no leaks are possible");
}

/// The `-O2` rows of `allocation_counts` only prove the hand-over if the call to `fill` is
/// still there: the inliner would otherwise fold it into `main`.
#[test]
fn fill_call_survives_o2() {
    for name in ["handover", "copies"] {
        let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
            .current_dir(RUN)
            .args(["-O2", "--emit", "ir"])
            .arg(format!("{name}.sisu"))
            .output()
            .expect("sisuc starts");
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let (_, after) = stdout
            .split_once("\n; after default<O2>\n")
            .unwrap_or_else(|| panic!("no `; after default<O2>` line in\n{stdout}"));
        let (_, main_onward) = after
            .split_once("define void @sisu.main()")
            .unwrap_or_else(|| panic!("{name}: no `sisu.main` in\n{after}"));
        let main = main_onward
            .split("\n}")
            .next()
            .expect("split yields one piece");
        assert!(main.contains("call ptr @sisu.fill("), "{name}: {main}");
    }
}

/// Program, stdout at k = 1000, stdout at k = 0, extra allocations.
const ALLOCATIONS: [(&str, &str, &str, i64); 3] = [
    // `big = fill(big, i, 1)` writes in place.
    ("handover", "999\n", "0\n", 0),
    // `saved` is live across each write, so each `fill` copies once.
    ("copies", "499500\n1000\n", "0\n0\n", 1000),
    // `for` over an array copies nothing.
    ("walk", "10000000\n", "0\n", 0),
];

#[test]
fn allocation_counts() {
    for o2 in [false, true] {
        for (name, stdout, base_stdout, extra) in ALLOCATIONS {
            assert_eq!(
                extra_allocs(name, o2),
                (stdout.to_string(), base_stdout.to_string(), extra),
                "{name} (o2: {o2})"
            );
        }
    }
}

/// Compiles `tests/programs/<name>.sisu` without `-O2` and runs it under a 256 KiB stack.
fn run_in_small_stack(name: &str) -> Output {
    let exe = compile_to_tmp(Path::new(PROGRAMS), name, false);
    Command::new("sh")
        .args(["-c", r#"ulimit -c 0 && ulimit -s 256 && exec "$0""#])
        .arg(exe)
        .output()
        .expect("sh starts")
}

/// Runs `<name>.sisu` under the small stack and checks that it exits 0 and prints `expected`.
fn assert_small_stack_prints(name: &str, expected: &str) {
    let out = run_in_small_stack(name);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{:?}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}

#[test]
fn longlist_frees_in_constant_stack() {
    assert_small_stack_prints("longlist", "1000000\n");
}

#[test]
fn right_skewed_tree_frees_in_constant_stack() {
    assert_small_stack_prints("rightskew", "1000000\n");
}

/// The control for `longlist_frees_in_constant_stack`: a drop that recurses once per object
/// overflows the same stack, so the limit is in force.
#[test]
fn deepchain_overflows_the_same_stack() {
    let out = run_in_small_stack("deepchain");
    assert_eq!(out.status.signal(), Some(11), "{:?}", out.status);
    // It dies in the drop, after `print`: stdout is line-buffered.
    assert_eq!(String::from_utf8_lossy(&out.stdout), "1\n");
}

/// Writes `source` to `<CARGO_TARGET_TMPDIR>/<name>.sisu` and compiles it from there, at
/// `-O2` if `o2`.
fn compile_tmp(name: &str, source: &str, o2: bool) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join(format!("{name}.sisu")), source).expect("writes");
    compile_to_tmp(dir, name, o2)
}

#[test]
fn panic_cases() {
    let cases = [
        (
            "sub_overflow",
            format!("fn main() {{\n{MIN}\n    print(min - 1)\n}}\n"),
            "3:11: integer overflow".to_string(),
        ),
        (
            "mul_overflow",
            format!("fn main() {{\n{MAX}\n    print(big * 2)\n}}\n"),
            "3:11: integer overflow".to_string(),
        ),
        (
            "neg_overflow",
            format!("fn main() {{\n{MIN}\n    print(-min)\n}}\n"),
            "3:11: integer overflow".to_string(),
        ),
        (
            "rem_zero",
            "fn main() {\n    let z = 0\n    print(7 % z)\n}\n".to_string(),
            "3:11: division by zero".to_string(),
        ),
        (
            "rem_overflow",
            format!("fn main() {{\n{MIN}\n    print(min % -1)\n}}\n"),
            "3:11: integer overflow".to_string(),
        ),
        (
            "compound_overflow",
            "fn main() {\n    let mut x = 9_223_372_036_854_775_807\n    x += 1\n    print(x)\n}\n"
                .to_string(),
            "3:5: integer overflow".to_string(),
        ),
        print_case("u8_add", "let a: u8 = 255", "a + 1", "integer overflow"),
        print_case("u8_sub", "let a: u8 = 0", "a - 1", "integer overflow"),
        print_case("u8_mul", "let a: u8 = 16", "a * a", "integer overflow"),
        print_case("u8_div_zero", "let a: u8 = 0", "7 / a", "division by zero"),
        print_case(
            "u8_convert",
            "let a = 300",
            "u8(a)",
            "value does not fit in `u8`",
        ),
        print_case(
            "u8_convert_256",
            "let a = 256",
            "u8(a)",
            "value does not fit in `u8`",
        ),
        print_case(
            "u8_convert_negative",
            "let a = -1",
            "u8(a)",
            "value does not fit in `u8`",
        ),
        sum_case("sum_overflow", "9_223_372_036_854_775_807, 1"),
        // The total fits, but the running sum overflows at the second element.
        sum_case("sum_overflow_midway", "9_223_372_036_854_775_807, 1, -1"),
        bounds_case(
            "bounds_at_length",
            "5",
            "index 5 out of bounds for length 5",
        ),
        bounds_case("bounds_above", "7", "index 7 out of bounds for length 5"),
        bounds_case(
            "bounds_negative",
            "-1",
            "index -1 out of bounds for length 5",
        ),
        // The inner index is checked at its own level, after the outer one.
        (
            "write_bounds",
            "fn main() {\n    let mut grid = [[false, false, false]]\n    grid[0][9] = true\n}\n"
                .to_string(),
            "3:13: index 9 out of bounds for length 3".to_string(),
        ),
        (
            "length_negative",
            "fn main() {\n    let n = -1\n    let xs = [0; n]\n    print(xs.len())\n}\n"
                .to_string(),
            "3:18: array length is negative".to_string(),
        ),
        // 16 + 9_223_372_036_854_775_791 bytes is exactly `i64::MAX`, above the largest
        // size that rounds up to the alignment of 8 within it.
        fill_case("length_too_large", "false", "9_223_372_036_854_775_791", 22),
        // 2^62 * 8 bytes overflows the multiply.
        fill_case("length_overflows", "0", "4_611_686_018_427_387_904", 18),
        // (2^61 - 1) * 8 bytes is 2^64 - 8, which fits; adding the 16-byte header wraps.
        fill_case("header_overflows", "0", "2_305_843_009_213_693_951", 18),
    ];
    for (name, source, position_and_message) in cases {
        for o2 in [false, true] {
            let out = Command::new(compile_tmp(name, &source, o2))
                .output()
                .expect("the program starts");
            assert_panics(
                &out,
                "",
                &format!("sisu: panic at {name}.sisu:{position_and_message}\n"),
            );
        }
    }
}

/// A `panic_cases` row whose panic is at `print(<expr>)` on line 3, column 11.
fn print_case(
    name: &'static str,
    binding: &str,
    expr: &str,
    message: &str,
) -> (&'static str, String, String) {
    (
        name,
        format!("fn main() {{\n    {binding}\n    print({expr})\n}}\n"),
        format!("3:11: {message}"),
    )
}

/// A `panic_cases` row whose `xs.sum()`, over the elements `elements`, overflows.
fn sum_case(name: &'static str, elements: &str) -> (&'static str, String, String) {
    print_case(
        name,
        &format!("let xs = [{elements}]"),
        "xs.sum()",
        "integer overflow",
    )
}

/// A `panic_cases` row for `[value; len]` on line 2, whose panic is at `len`, in column `col`.
fn fill_case(
    name: &'static str,
    value: &str,
    len: &str,
    col: usize,
) -> (&'static str, String, String) {
    (
        name,
        format!("fn main() {{\n    let xs = [{value}; {len}]\n    print(xs.len())\n}}\n"),
        format!("2:{col}: array is too large"),
    )
}

/// The largest block the size checks allow, `i64::MAX` rounded down to the alignment of 8,
/// passes them and reaches the allocator, which cannot provide it: `sisu_alloc` calls
/// `handle_alloc_error` rather than aborting on an invalid layout, which prints nothing.
#[test]
fn largest_array_reaches_the_allocator() {
    // 16 + 9_223_372_036_854_775_784 bytes is 9_223_372_036_854_775_800.
    let source =
        "fn main() {\n    let xs = [false; 9_223_372_036_854_775_784]\n    print(xs.len())\n}\n";
    for o2 in [false, true] {
        let out = Command::new(compile_tmp("largest_array", source, o2))
            .output()
            .expect("the program starts");
        assert_eq!(out.status.signal(), Some(6), "{:?}", out.status);
        // A backtrace note may follow, depending on `RUST_BACKTRACE`.
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            stderr.lines().next(),
            Some("memory allocation of 9223372036854775800 bytes failed"),
            "{stderr}"
        );
    }
}

/// A `panic_cases` row that reads `xs[i]` from a 5-element `xs`, with `i` bound to `index`;
/// the panic is at `i`, on line 4, column 14.
fn bounds_case(name: &'static str, index: &str, message: &str) -> (&'static str, String, String) {
    (
        name,
        format!(
            "fn main() {{\n    let xs = [1, 2, 3, 4, 5]\n    let i = {index}\n    print(xs[i])\n}}\n"
        ),
        format!("4:14: {message}"),
    )
}

/// Prints `1` and exits 0, but leaks: an object that refers to itself is never freed. Building
/// the cycle in a callee keeps the pointer out of `main`'s registers.
const LEAKS_A_CYCLE: &str = "class Node {\n    mut next: Node?\n}\n\nfn leak() {\n    let a = Node(next: None)\n    a.next = a\n}\n\nfn main() {\n    leak()\n    print(1)\n}\n";

/// Proves the harness catches a leak.
#[test]
fn valgrind_reports_a_cycle() {
    let exe = compile_tmp("cycle", LEAKS_A_CYCLE, false);
    let out = Command::new(&exe).output().expect("the program starts");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "1\n");
    let checked = support::valgrind(&exe);
    let stderr = String::from_utf8_lossy(&checked.stderr);
    assert_eq!(checked.status.code(), Some(1), "{stderr}");
    // A loss record, not the summary line, which also prints `definitely lost: 0 bytes`.
    assert!(
        stderr.contains("are definitely lost") || stderr.contains("are indirectly lost"),
        "{stderr}"
    );
}

/// Writes `source` to `<CARGO_TARGET_TMPDIR>/<name>.sisu`, builds it with `--rc-checks`, at
/// `-O2` if `o2`, into `<name>-rc` or `<name>-rc-O2`, and runs it.
fn run_rc_checked(name: &str, source: &str, o2: bool) -> Output {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join(format!("{name}.sisu")), source).expect("writes");
    let (exe, flags): (_, &[&str]) = if o2 {
        (format!("{name}-rc-O2"), &["-O2", "--rc-checks"])
    } else {
        (format!("{name}-rc"), &["--rc-checks"])
    };
    let exe = dir.join(exe);
    support::compile(dir, name, &exe, flags);
    Command::new(exe).output().expect("the program starts")
}

/// The cycle Valgrind reports, `--rc-checks` reports too, with or without `-O2`: one object is
/// live at exit.
#[test]
fn rc_checks_report_a_cycle() {
    for o2 in [false, true] {
        let out = run_rc_checked("rc_cycle", LEAKS_A_CYCLE, o2);
        assert_eq!(out.status.code(), Some(102), "o2: {o2}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "1\n");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            "sisu: rc check: live objects at exit: 1\n"
        );
    }
}

/// A panic exits 101 without releasing, so the object `b` still holds is no leak to report.
#[test]
fn rc_checks_ignore_a_panic() {
    let out = run_rc_checked(
        "rc_panic",
        "class Box {\n    v: i64\n}\n\nfn main() {\n    let b = Box(v: 1)\n    let z = 0\n    print(1 / z)\n    print(b.v)\n}\n",
        false,
    );
    assert_panics(
        &out,
        "",
        "sisu: panic at rc_panic.sisu:8:11: division by zero\n",
    );
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
        false,
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
        let out = support::sisuc(dir, "overwrite_input.sisu", Path::new(output), &[])
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
    let out = support::sisuc(dir, "keep_object.sisu", &dir.join("keep_object"), &[])
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
    let out = support::sisuc(dir, "link_fails.sisu", &output, &[])
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
    let out = support::sisuc(dir, &format!("{name}.sisu"), &exe, &[])
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
        "fn main() {\n    let mut count = 0\n    print(count)\n}\n",
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with("warning: `count` is never reassigned")
    );
    let run = Command::new(exe).output().expect("the program starts");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "0\n");
}

/// A program that prints `1`.
const PRINTS_1: &str = "fn main() {\n    print(1)\n}\n";
/// A program that prints `1`, then panics at 4:11 with `division by zero`. It prints first so
/// that a write of its stdout would change an empty `.out`.
const PRINTS_1_THEN_PANICS: &str =
    "fn main() {\n    print(1)\n    let z = 0\n    print(1 / z)\n}\n";

/// Writes each `(path, contents)` under a fresh `<CARGO_TARGET_TMPDIR>/fixtures/<test>/` and
/// returns that directory.
fn fixture(test: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("fixtures")
        .join(test);
    // It is absent on the first run, so a failure here is expected and harmless.
    let _ = fs::remove_dir_all(&dir);
    for (path, contents) in files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().expect("a fixture file should have a parent"))
            .expect("creates the fixture directory");
        fs::write(path, contents).expect("writes the fixture file");
    }
    dir
}

/// Runs `f`, which should panic, and returns its panic message.
fn panic_message(f: impl FnOnce()) -> String {
    let payload = panic::catch_unwind(AssertUnwindSafe(f)).expect_err("the call should panic");
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .expect("a panic message should be a string")
}

#[test]
fn bless_refuses_to_create_a_missing_out() {
    let dir = fixture(
        "bless_refuses_to_create_a_missing_out",
        &[("a.sisu", PRINTS_1)],
    );
    let message = panic_message(|| support::run(&dir.join("a.sisu"), true));
    assert!(message.contains("is missing; create it empty"), "{message}");
    assert!(!dir.join("a.out").exists());
}

#[test]
fn bless_never_writes_after_a_wrong_exit() {
    let dir = fixture(
        "bless_never_writes_after_a_wrong_exit",
        &[("a.sisu", PRINTS_1_THEN_PANICS), ("a.out", "")],
    );
    let message = panic_message(|| support::run(&dir.join("a.sisu"), true));
    assert!(message.contains("Some(101)"), "{message}");
    assert_eq!(
        fs::read_to_string(dir.join("a.out")).expect("reads a.out"),
        ""
    );
}

/// A run that passes natively but fails under Valgrind writes nothing.
#[test]
fn bless_never_writes_after_a_valgrind_failure() {
    let dir = fixture(
        "bless_never_writes_after_a_valgrind_failure",
        &[("a.sisu", LEAKS_A_CYCLE), ("a.out", "wrong\n")],
    );
    let message = panic_message(|| support::run(&dir.join("a.sisu"), true));
    assert!(message.contains("LEAK SUMMARY"), "{message}");
    assert_eq!(
        fs::read_to_string(dir.join("a.out")).expect("reads a.out"),
        "wrong\n"
    );
}

/// Bless writes a `run_fail` case's files all or nothing: a missing `.err` leaves `.out` as it was.
#[test]
fn bless_writes_neither_file_when_one_is_missing() {
    let dir = fixture(
        "bless_writes_neither_file_when_one_is_missing",
        &[("a.sisu", PRINTS_1_THEN_PANICS), ("a.out", "")],
    );
    let message = panic_message(|| support::run_fail(&dir.join("a.sisu"), true));
    assert!(message.contains("a.err is missing"), "{message}");
    assert_eq!(
        fs::read_to_string(dir.join("a.out")).expect("reads a.out"),
        ""
    );
    assert!(!dir.join("a.err").exists());
}

/// A program that compiles is no compile-fail case, whatever its `.err` holds.
#[test]
fn compile_fail_rejects_a_valid_program() {
    let dir = fixture(
        "compile_fail_rejects_a_valid_program",
        &[("a.sisu", "fn main() {}\n"), ("a.err", "")],
    );
    let message = panic_message(|| support::compile_fail(&dir.join("a.sisu"), true));
    assert!(message.contains("Some(0)"), "{message}");
    assert_eq!(
        fs::read_to_string(dir.join("a.err")).expect("reads a.err"),
        ""
    );
}

/// Without bless a wrong `.out` fails and stays as it was; bless rewrites it, and the run then
/// passes without bless.
#[test]
fn bless_rewrites_an_existing_out() {
    let dir = fixture(
        "bless_rewrites_an_existing_out",
        &[("a.sisu", PRINTS_1), ("a.out", "wrong\n")],
    );
    let source = dir.join("a.sisu");
    let message = panic_message(|| support::run(&source, false));
    assert!(message.contains("SISU_BLESS=1"), "{message}");
    assert_eq!(
        fs::read_to_string(dir.join("a.out")).expect("reads a.out"),
        "wrong\n"
    );
    support::run(&source, true);
    assert_eq!(
        fs::read_to_string(dir.join("a.out")).expect("reads a.out"),
        "1\n"
    );
    support::run(&source, false);
}

#[test]
#[should_panic(expected = "ghost.out")]
fn orphan_out_fails() {
    let dir = fixture("orphan_out_fails", &[("ghost.out", "1\n")]);
    support::orphan_check(&dir.join("ghost.out"));
}

/// Same-named cases in different directories, run at once, must not share an executable: one
/// in each mode, and two whose paths differ only in `a_b` against `a/b`.
#[test]
fn same_name_in_two_modes() {
    let dir = fixture(
        "same_name_in_two_modes",
        &[
            ("run/dup.sisu", PRINTS_1),
            ("run/dup.out", "1\n"),
            ("run/a_b/dup.sisu", PRINTS_1),
            ("run/a_b/dup.out", "1\n"),
            ("run/a/b/dup.sisu", PRINTS_1_THEN_PANICS),
            ("run/a/b/dup.out", "1\n"),
            (
                "run/a/b/dup.err",
                "sisu: panic at dup.sisu:4:11: division by zero\n",
            ),
            ("run-fail/dup.sisu", PRINTS_1_THEN_PANICS),
            ("run-fail/dup.out", "1\n"),
            (
                "run-fail/dup.err",
                "sisu: panic at dup.sisu:4:11: division by zero\n",
            ),
        ],
    );
    thread::scope(|scope| {
        scope.spawn(|| support::run(&dir.join("run/dup.sisu"), false));
        scope.spawn(|| support::run_fail(&dir.join("run-fail/dup.sisu"), false));
        scope.spawn(|| support::run(&dir.join("run/a_b/dup.sisu"), false));
        scope.spawn(|| support::run_fail(&dir.join("run/a/b/dup.sisu"), false));
    });
}

/// Each program builds to the same bytes twice, at two different output paths.
#[test]
fn builds_are_deterministic() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    for name in ["life", "optionals", "arrays"] {
        let [a, b] = ["a", "b"].map(|build| {
            let exe = dir.join(format!("deterministic-{name}-{build}"));
            support::compile(Path::new(RUN), name, &exe, &["-O2"]);
            fs::read(&exe).expect("reads the build")
        });
        assert!(a == b, "{name}.sisu: two -O2 builds differ");
    }
}

/// Generated `i64`, `u8` and `bool` expressions, which Sisu at O0 and `-O2` must print as Rust
/// computes them.
mod arithmetic {
    use std::fmt;
    use std::process::Command;

    use super::compile_tmp;

    /// Xorshift64: fixed seeds give the same expressions on every run.
    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn pick<T: Copy>(&mut self, items: &[T]) -> T {
            let len = u64::try_from(items.len()).expect("a slice length fits u64");
            items[usize::try_from(self.next() % len).expect("an index below a length fits usize")]
        }
    }

    #[derive(Clone, Copy)]
    enum Ty {
        I64,
        U8,
        Bool,
    }

    #[derive(Clone, Copy)]
    enum Value {
        I64(i64),
        U8(u8),
        Bool(bool),
    }

    impl Value {
        /// An integer as `i64`, which holds every `u8` and keeps its order.
        fn int(self) -> i64 {
            match self {
                Value::I64(n) => n,
                Value::U8(n) => n.into(),
                Value::Bool(_) => panic!("a generated integer operand is a bool"),
            }
        }

        fn truth(self) -> bool {
            match self {
                Value::Bool(b) => b,
                _ => panic!("a generated condition is not a bool"),
            }
        }
    }

    impl fmt::Display for Value {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Value::I64(n) => write!(f, "{n}"),
                Value::U8(n) => write!(f, "{n}"),
                Value::Bool(b) => write!(f, "{b}"),
            }
        }
    }

    const ARITH: [&str; 5] = ["+", "-", "*", "/", "%"];
    const COMPARE: [&str; 6] = ["==", "!=", "<", "<=", ">", ">="];
    /// Literals near the overflow edges, beside the small ones.
    const BIG: [i64; 5] = [i64::MAX, i64::MAX - 1, 1 << 32, 1 << 62, 3_037_000_499];

    enum Expr {
        Lit(Value),
        Neg(Box<Expr>),
        /// `+ - * / %`, both sides `i64` or both `u8`.
        Arith(&'static str, Box<Expr>, Box<Expr>),
        /// `== != < <= > >=`, both sides `i64` or both `u8`.
        Compare(&'static str, Box<Expr>, Box<Expr>),
        /// `&&` or `||`.
        Logic(&'static str, Box<Expr>, Box<Expr>),
        If(Box<Expr>, Box<Expr>, Box<Expr>),
    }

    /// `a op b` with Rust's checked arithmetic: truncating `/`, `%` with the dividend's sign.
    fn arith(op: &str, a: i64, b: i64) -> Option<i64> {
        match op {
            "+" => a.checked_add(b),
            "-" => a.checked_sub(b),
            "*" => a.checked_mul(b),
            "/" => a.checked_div(b),
            "%" => a.checked_rem(b),
            _ => unreachable!("not an arithmetic operator: {op}"),
        }
    }

    impl Expr {
        /// The Sisu source, every operator in parentheses so no precedence or chaining applies.
        fn render(&self) -> String {
            match self {
                Expr::Lit(Value::I64(n)) if *n < 0 => format!("(-{})", n.unsigned_abs()),
                Expr::Lit(Value::U8(n)) => format!("u8({n})"),
                Expr::Lit(v) => v.to_string(),
                Expr::Neg(e) => format!("(-{})", e.render()),
                Expr::Arith(op, l, r) | Expr::Compare(op, l, r) | Expr::Logic(op, l, r) => {
                    format!("({} {op} {})", l.render(), r.render())
                }
                Expr::If(c, a, b) => format!(
                    "(if {} {{ {} }} else {{ {} }})",
                    c.render(),
                    a.render(),
                    b.render()
                ),
            }
        }

        /// The value, or `None` where Sisu panics: overflow or division by zero. `&&`, `||`
        /// and `if` evaluate only what Sisu does.
        fn eval(&self) -> Option<Value> {
            Some(match self {
                Expr::Lit(v) => *v,
                Expr::Neg(e) => Value::I64(e.eval()?.int().checked_neg()?),
                Expr::Arith(op, l, r) => {
                    let (l, r) = (l.eval()?, r.eval()?);
                    let n = arith(op, l.int(), r.int())?;
                    match l {
                        Value::U8(_) => Value::U8(u8::try_from(n).ok()?),
                        _ => Value::I64(n),
                    }
                }
                Expr::Compare(op, l, r) => {
                    let (l, r) = (l.eval()?.int(), r.eval()?.int());
                    Value::Bool(match *op {
                        "==" => l == r,
                        "!=" => l != r,
                        "<" => l < r,
                        "<=" => l <= r,
                        ">" => l > r,
                        ">=" => l >= r,
                        _ => unreachable!("not a comparison: {op}"),
                    })
                }
                Expr::Logic(op, l, r) => {
                    let l = l.eval()?.truth();
                    // `&&` stops at false, `||` at true.
                    if l == (*op == "||") {
                        Value::Bool(l)
                    } else {
                        r.eval()?
                    }
                }
                Expr::If(c, a, b) => {
                    if c.eval()?.truth() {
                        a.eval()?
                    } else {
                        b.eval()?
                    }
                }
            })
        }

        /// An expression of type `ty`, at most `depth` operators deep.
        fn generate(rng: &mut XorShift, ty: Ty, depth: u32) -> Expr {
            let boxed = |rng: &mut XorShift, ty| Box::new(Expr::generate(rng, ty, depth - 1));
            // A leaf at depth 0, and one time in four above it.
            let choice = if depth == 0 || rng.next().is_multiple_of(4) {
                0
            } else {
                1 + rng.next() % 3
            };
            match (ty, choice) {
                (Ty::I64, 0) => Expr::Lit(Value::I64(if rng.next().is_multiple_of(4) {
                    let big = rng.pick(&BIG);
                    if rng.next().is_multiple_of(2) {
                        big
                    } else {
                        -big
                    }
                } else {
                    i64::try_from(rng.next() % 41).expect("below 41 fits i64") - 20
                })),
                (Ty::U8, 0) => Expr::Lit(Value::U8(rng.pick(&[0, 1, 2, 3, 7, 16, 100, 200, 255]))),
                (Ty::Bool, 0) => Expr::Lit(Value::Bool(rng.next().is_multiple_of(2))),
                (Ty::I64, 1) => Expr::Neg(boxed(rng, Ty::I64)),
                (Ty::I64 | Ty::U8, 1 | 2) => {
                    Expr::Arith(rng.pick(&ARITH), boxed(rng, ty), boxed(rng, ty))
                }
                (Ty::Bool, 1) => {
                    let operands = rng.pick(&[Ty::I64, Ty::U8]);
                    Expr::Compare(
                        rng.pick(&COMPARE),
                        boxed(rng, operands),
                        boxed(rng, operands),
                    )
                }
                (Ty::Bool, 2) => {
                    Expr::Logic(rng.pick(&["&&", "||"]), boxed(rng, ty), boxed(rng, ty))
                }
                _ => Expr::If(boxed(rng, Ty::Bool), boxed(rng, ty), boxed(rng, ty)),
            }
        }
    }

    /// Fifty expressions per seed, each regenerated until Rust can evaluate it, printed by one
    /// program, which both builds must run to exactly the lines Rust computes.
    #[test]
    fn generated_arithmetic_matches_rust() {
        for seed in 1..=4 {
            let mut rng = XorShift(seed);
            let exprs: Vec<(String, Value)> = (0..50)
                .map(|i| {
                    let ty = [Ty::I64, Ty::U8, Ty::Bool][i % 3];
                    loop {
                        let e = Expr::generate(&mut rng, ty, 4);
                        if let Some(value) = e.eval() {
                            break (e.render(), value);
                        }
                    }
                })
                .collect();
            let prints: Vec<String> = exprs
                .iter()
                .map(|(src, _)| format!("    print({src})"))
                .collect();
            let source = format!("fn main() {{\n{}\n}}\n", prints.join("\n"));
            for o2 in [false, true] {
                let exe = compile_tmp(&format!("generated_{seed}"), &source, o2);
                let out = Command::new(&exe).output().expect("the program starts");
                let stdout = String::from_utf8_lossy(&out.stdout);
                let lines: Vec<&str> = stdout.lines().collect();
                let first_wrong = exprs
                    .iter()
                    .enumerate()
                    .find(|(i, (_, value))| lines.get(*i) != Some(&value.to_string().as_str()));
                if let Some((i, (src, value))) = first_wrong {
                    panic!(
                        "seed {seed}, {} build, expression {i}: `{src}` printed {:?}, Rust \
                         says {value}; exit {:?}, stderr:\n{}",
                        if o2 { "-O2" } else { "O0" },
                        lines.get(i),
                        out.status,
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
                assert_eq!(lines.len(), exprs.len(), "seed {seed}: extra output");
                assert_eq!(out.status.code(), Some(0), "seed {seed}: {:?}", out.status);
            }
        }
    }
}
