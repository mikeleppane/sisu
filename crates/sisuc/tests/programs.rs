//! End to end: `sisuc` compiles each program in `tests/programs`, and the program
//! prints the right output or panics with the right message.

use std::fs::{self, File};
use std::os::unix::process::ExitStatusExt;
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

/// One test per program in `tests/programs` that exits 0, named after the program unless it
/// gives `test = "file"`, plus `OUT_FILE_PROGRAMS`, the list `every_program_has_a_run` checks.
macro_rules! out_file_tests {
    (@file $test:ident $file:literal) => {
        $file
    };
    (@file $test:ident) => {
        stringify!($test)
    };
    ($($test:ident $(= $file:literal)?),* $(,)?) => {
        const OUT_FILE_PROGRAMS: &[&str] = &[$(out_file_tests!(@file $test $($file)?)),*];
        $(
            #[test]
            fn $test() {
                assert_prints_out_file(out_file_tests!(@file $test $($file)?));
            }
        )*
    };
}

out_file_tests! {
    hello,
    fib,
    semantics,
    primes,
    loops,
    break_continue,
    short_circuit,
    loop_return,
    bool_var,
    objects,
    ownership,
    field_order,
    unwrap,
    list,
    for_range_releases_the_body_tail = "for_range_drop",
    drops,
    tree,
    optionals,
    bytes,
    unsigned,
    arrays,
    empty_arrays = "empty",
    values,
    life,
    moves,
    methods,
    sieve,
    handover,
    copies,
    walk,
}

/// The programs the small-stack tests run. They skip `assert_prints_out_file`: a million
/// objects under Valgrind is too slow, and `deepchain` overflows on purpose.
const SMALL_STACK: [&str; 3] = ["longlist", "rightskew", "deepchain"];

/// Each program in `tests/programs` has exactly one kind of run: an `.out` file and a test from
/// `out_file_tests!`, a `PANICS` row, or a small-stack test.
#[test]
fn every_program_has_a_run() {
    let panics = PANICS.map(|(name, _, _)| name);
    for entry in fs::read_dir(PROGRAMS).expect("tests/programs should be readable") {
        let path = entry.expect("a directory entry should be readable").path();
        // Tools may leave a directory here, such as `.claude/`; only files are programs.
        if path.is_dir() {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("a program's name should be UTF-8");
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("sisu") => {
                let runs = [
                    OUT_FILE_PROGRAMS.contains(&name),
                    panics.contains(&name),
                    SMALL_STACK.contains(&name),
                ];
                assert_eq!(
                    runs.iter().filter(|run| **run).count(),
                    1,
                    "{name}.sisu should be in exactly one of `out_file_tests!`, `PANICS` and \
                     `SMALL_STACK`"
                );
            }
            Some("out") => assert!(
                OUT_FILE_PROGRAMS.contains(&name),
                "{name}.out should have a test in `out_file_tests!`"
            ),
            _ => panic!("{} is neither a program nor an `.out` file", path.display()),
        }
    }
    for name in OUT_FILE_PROGRAMS {
        assert!(
            Path::new(PROGRAMS).join(format!("{name}.out")).exists(),
            "{name} is in `out_file_tests!` but has no `{name}.out`"
        );
    }
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
    let out = valgrind(exe);
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
    let source = fs::read_to_string(Path::new(PROGRAMS).join(format!("{name}.sisu")))
        .expect("reads the program");
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
            .current_dir(PROGRAMS)
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
    let exe = compile(Path::new(PROGRAMS), name, false);
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

/// Writes `source` to `<CARGO_TARGET_TMPDIR>/<name>.sisu` and compiles it from there, at
/// `-O2` if `o2`.
fn compile_tmp(name: &str, source: &str, o2: bool) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join(format!("{name}.sisu")), source).expect("writes");
    compile(dir, name, o2)
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
            "fn main() {\n    var x = 9_223_372_036_854_775_807\n    x += 1\n    print(x)\n}\n"
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
            "fn main() {\n    var grid = [[false, false, false]]\n    grid[0][9] = true\n}\n"
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

/// Proves the harness catches a leak: an object that refers to itself is never freed.
/// Building the cycle in a callee keeps the pointer out of `main`'s registers.
#[test]
fn valgrind_reports_a_cycle() {
    let exe = compile_tmp(
        "cycle",
        "class Node {\n    var next: Node?\n}\n\nfn leak() {\n    let a = Node(next: None)\n    a.next = a\n}\n\nfn main() {\n    leak()\n    print(1)\n}\n",
        false,
    );
    let out = Command::new(&exe).output().expect("the program starts");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "1\n");
    let checked = valgrind(&exe);
    let stderr = String::from_utf8_lossy(&checked.stderr);
    assert_eq!(checked.status.code(), Some(1), "{stderr}");
    // A loss record, not the summary line, which also prints `definitely lost: 0 bytes`.
    assert!(
        stderr.contains("are definitely lost") || stderr.contains("are indirectly lost"),
        "{stderr}"
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
