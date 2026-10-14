//! Helpers shared by the end-to-end test binaries.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// `sisuc <flags> <input> <output>`, run from `dir`, after asserting the runtime archive exists.
pub(crate) fn sisuc(dir: &Path, input: &str, output: &Path, flags: &[&str]) -> Command {
    let sisuc = Path::new(env!("CARGO_BIN_EXE_sisuc"));
    assert!(
        sisuc.with_file_name("libsisu_runtime.a").exists(),
        "libsisu_runtime.a is missing: run the tests with `cargo nextest run`, whose setup \
         script builds it, or run `cargo build -p sisu-runtime`"
    );
    let mut command = Command::new(sisuc);
    command.current_dir(dir).args(flags).arg(input).arg(output);
    command
}

/// Compiles `<dir>/<name>.sisu`, run from `dir`, into `exe`, passing `flags` to `sisuc`.
pub(crate) fn compile(dir: &Path, name: &str, exe: &Path, flags: &[&str]) {
    let out = sisuc(dir, &format!("{name}.sisu"), exe, flags)
        .output()
        .expect("sisuc starts");
    assert_eq!(
        out.status.code(),
        Some(0),
        "sisuc failed on {name}.sisu:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Runs `exe` under Valgrind; any leak or memory error makes it exit 1.
pub(crate) fn valgrind(exe: &Path) -> Output {
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

/// Checks a `tests/run` case: both builds differ, exit 0, agree on stdout and on stderr, and
/// print the same under Valgrind, which finds no leak or memory error, and in a `--rc-checks`
/// build, which finds no count fault or live object at exit; then that stdout is `<name>.out`.
/// Stderr is empty, unless `<name>.err` exists: then the stderr of the plain, `-O2` and
/// `--rc-checks` runs is that file. The Valgrind runs' stderr holds Valgrind's own log, so it is
/// never compared.
/// Each `// o2-calls:` marker in the case must hold in the `-O2` IR (see `O2_CALLS`).
/// With `bless`, rewrites `<name>.out`, and `<name>.err` when it exists, instead, after every
/// other check passed.
pub(crate) fn run(path: &Path, bless: bool) {
    let [o0, o2, rc] = build(path, [O0, O2, RC_CHECKS]);
    assert!(
        fs::read(&o0).expect("reads the plain build")
            != fs::read(&o2).expect("reads the -O2 build"),
        "{}: -O2 build is identical to the plain build",
        path.display()
    );
    o2_calls(path);
    let err_path = path.with_extension("err");
    let expects_stderr = err_path.exists();
    let [out0, out2] = [&o0, &o2].map(|exe| {
        let out = Command::new(exe).output().expect("the program starts");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}: {:?}\n{stderr}",
            exe.display(),
            out.status
        );
        if !expects_stderr {
            assert_eq!(
                stderr,
                "",
                "{}: writes to stderr, and the case has no `.err` file",
                exe.display()
            );
        }
        out
    });
    let stdout = agreed(path, "stdout", &out0.stdout, &out2.stdout);
    let stderr = agreed(path, "stderr", &out0.stderr, &out2.stderr);
    for exe in [o0, o2] {
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
            stdout,
            "{}: {valgrind_stderr}",
            exe.display()
        );
    }
    let checked = Command::new(&rc).output().expect("the program starts");
    let rc_stderr = String::from_utf8_lossy(&checked.stderr);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "{}: {:?}\n{rc_stderr}",
        rc.display(),
        checked.status
    );
    assert_eq!(rc_stderr, stderr, "{}", rc.display());
    assert_eq!(
        String::from_utf8_lossy(&checked.stdout),
        stdout,
        "{}",
        rc.display()
    );
    // Last, so that a run failing any check above writes nothing.
    let mut files = vec![(path.with_extension("out"), stdout)];
    if expects_stderr {
        files.push((err_path, stderr));
    }
    expect_files(&files, bless);
}

/// Checks a `tests/run-fail` case: both builds exit 101, agree, and print `<name>.out` on stdout
/// and `<name>.err` on stderr. With `bless`, rewrites both files instead. No Valgrind: a panic
/// exits without releasing.
pub(crate) fn run_fail(path: &Path, bless: bool) {
    let [out0, out2] = build(path, [O0, O2]).map(|exe| {
        let out = Command::new(&exe).output().expect("the program starts");
        assert_eq!(
            out.status.code(),
            Some(101),
            "{}: {:?}\n{}",
            exe.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        out
    });
    let stdout = agreed(path, "stdout", &out0.stdout, &out2.stdout);
    let stderr = agreed(path, "stderr", &out0.stderr, &out2.stderr);
    expect_files(
        &[
            (path.with_extension("out"), stdout),
            (path.with_extension("err"), stderr),
        ],
        bless,
    );
}

/// Checks a `tests/diagnostics` case: `sisuc --check <name>.sisu`, run from the source's
/// directory, reports at least one diagnostic, prints nothing on stdout, and exits 1 when a
/// stderr line starts with `error`, else 0. A case named after a code (`E0001.sisu`,
/// `E0001-field.sisu`) reports that code. `sisuc --format json --check` exits the same, and each
/// of its stderr lines is a JSON object, the summary last. Then the two stderrs are `<name>.err`
/// and `<name>.json`; with `bless`, rewrites both files instead.
pub(crate) fn diagnostics(path: &Path, bless: bool) {
    let dir = path.parent().expect("a case should sit in a directory");
    let name = path.file_name().expect("a case should have a file name");
    let sisuc = Path::new(env!("CARGO_BIN_EXE_sisuc"));
    let out = Command::new(sisuc)
        .current_dir(dir)
        .arg("--check")
        .arg(name)
        .output()
        .expect("sisuc starts");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let display = path.display();
    assert!(!stderr.is_empty(), "{display} reports no diagnostic");
    let failed = stderr.lines().any(|l| l.starts_with("error"));
    assert_eq!(
        out.status.code(),
        Some(i32::from(failed)),
        "{display}: {stderr}"
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "", "{display}");
    let stem = path
        .file_stem()
        .and_then(OsStr::to_str)
        .expect("a case's name should be UTF-8");
    let code = stem.split_once('-').map_or(stem, |(code, _)| code);
    if is_code(code) {
        assert!(
            stderr.contains(&format!("[{code}]")),
            "{display} does not report {code}: {stderr}"
        );
    }
    let out = Command::new(sisuc)
        .current_dir(dir)
        .args(["--format", "json", "--check"])
        .arg(name)
        .output()
        .expect("sisuc starts");
    let json = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        out.status.code(),
        Some(i32::from(failed)),
        "{display} --format json: {json}"
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "", "{display}");
    assert!(
        json.lines()
            .all(|l| l.starts_with(r#"{"version":1,"kind":""#)),
        "{display}: a stderr line is not a JSON object: {json}"
    );
    assert!(
        json.lines()
            .last()
            .is_some_and(|l| l.contains(r#""kind":"summary""#)),
        "{display}: the summary is not last: {json}"
    );
    expect_files(
        &[
            (path.with_extension("err"), stderr),
            (path.with_extension("json"), json),
        ],
        bless,
    );
}

/// Whether `text` is a diagnostic code: `E` or `W`, then four digits.
fn is_code(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 5 && matches!(bytes[0], b'E' | b'W') && bytes[1..].iter().all(u8::is_ascii_digit)
}

/// Passes for a `.sisu` file, and for a file whose extension is one of `expectations` (the
/// files the directory's mode reads) beside a `.sisu` of the same name; panics on any other
/// file, naming it. It also panics, naming it, on any hidden entry beside `path`: the harness
/// never lists one, so a stray `.ghost.out` would otherwise go unseen.
pub(crate) fn orphan_check(path: &Path, expectations: &[&str]) {
    let belongs = match path.extension().and_then(OsStr::to_str) {
        Some("sisu") => true,
        Some(ext) if expectations.contains(&ext) => path.with_extension("sisu").exists(),
        _ => false,
    };
    assert!(
        belongs,
        "{} is neither a `.sisu` case nor the {expectations:?} file of one",
        path.display()
    );
    let dir = path
        .parent()
        .expect("a case file should sit in a directory");
    // shortcut: looks only beside `path`, so a directory of nothing but hidden files is unseen;
    // walk the whole root if case directories gain subdirectories.
    for entry in fs::read_dir(dir).expect("the case directory should be readable") {
        let sibling = entry.expect("a directory entry should be readable").path();
        assert!(
            !sibling
                .file_name()
                .is_some_and(|name| name.as_encoded_bytes().starts_with(b".")),
            "{} is hidden, so no case reads it; remove it",
            sibling.display()
        );
    }
}

/// The builds `build` makes: the executable's name in the case directory, and `sisuc`'s flags.
type Build = (&'static str, &'static [&'static str]);
const O0: Build = ("O0", &[]);
const O2: Build = ("O2", &["-O2"]);
const RC_CHECKS: Build = ("rc", &["--rc-checks"]);

/// Compiles `path` once for each of `builds` into its `case_dir`, and returns the executables.
fn build<const N: usize>(path: &Path, builds: [Build; N]) -> [PathBuf; N] {
    let dir = path.parent().expect("a case should sit in a directory");
    let name = path
        .file_stem()
        .and_then(OsStr::to_str)
        .expect("a case's name should be UTF-8");
    let out = case_dir(path);
    builds.map(|(file, flags)| {
        let exe = out.join(file);
        compile(dir, name, &exe, flags);
        exe
    })
}

/// A fresh, empty `<CARGO_TARGET_TMPDIR>/cases/<source path>/`. The whole path, nested, keeps
/// same-named cases in different directories apart.
fn case_dir(source: &Path) -> PathBuf {
    // An absolute path would replace the base in `join`, so it goes in without its leading `/`.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cases")
        .join(source.strip_prefix("/").unwrap_or(source));
    // It is absent on the first run, so a failure here is expected and harmless.
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("creates the case directory");
    dir
}

/// The marker a `tests/run` case puts on its own line to prove a call boundary: with
/// `// o2-calls: <caller> <callee> <count>`, the `-O2` IR of `<caller>` holds exactly `<count>`
/// calls to `<callee>`, so the inliner kept the calls the case relies on. Every function but
/// the C `main` is internal, so `-O2` would inline a caller with one call site and the caller
/// would vanish from the IR. The check therefore builds once per distinct caller, with
/// `SISU_KEEP_FUNCTIONS` set to that caller alone, which marks it `noinline` for
/// `sisuc --emit ir` and nothing else, and counts in that build. One caller at a time, so that
/// keeping one caller does not also keep a callee that another marker names as its caller. A
/// caller that takes parameters is rejected: kept as a function of its own, it could hold a
/// call that its real call sites decide away.
const O2_CALLS: &str = "// o2-calls:";

/// The text of the function `name` in `ir`, from its header line to its closing brace, if `ir`
/// defines it.
fn function_text<'a>(ir: &'a str, name: &str) -> Option<&'a str> {
    let header = format!("@sisu.{name}(");
    ir.split("\ndefine ")
        .skip(1)
        .find(|function| function.lines().next().is_some_and(|l| l.contains(&header)))
        .and_then(|function| function.split("\n}").next())
}

/// `sisuc -O2 --emit ir` on the case `path` with `SISU_KEEP_FUNCTIONS=caller`, as the module
/// before and the module after the pipeline.
fn o2_ir(path: &Path, caller: &str) -> (String, String) {
    let dir = path.parent().expect("a case should sit in a directory");
    let name = path.file_name().expect("a case should have a file name");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .env("SISU_KEEP_FUNCTIONS", caller)
        .args(["-O2", "--emit", "ir"])
        .arg(name)
        .output()
        .expect("sisuc starts");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let ir = String::from_utf8_lossy(&out.stdout);
    let (before, after) = ir
        .split_once("\n; after default<O2>\n")
        .unwrap_or_else(|| panic!("{}: no `; after default<O2>` line", path.display()));
    (before.to_string(), after.to_string())
}

/// Checks each `O2_CALLS` marker in `path` against `sisuc -O2 --emit ir`, built once per caller.
/// The marked caller is measured as a standalone function. In a real build it is inlined, and a
/// call to a callee that LLVM infers as `memory(none)` or `readonly` can then be merged with an
/// identical call at the call site. So mark calls to callees that write memory.
fn o2_calls(path: &Path) {
    let source = fs::read_to_string(path).expect("reads the case");
    let markers: Vec<[&str; 3]> = source
        .lines()
        .filter_map(|line| line.trim().strip_prefix(O2_CALLS))
        .map(|marker| {
            marker
                .split_whitespace()
                .collect::<Vec<_>>()
                .try_into()
                .unwrap_or_else(|_| {
                    panic!(
                        "{}: `{O2_CALLS}{marker}` is not `<caller> <callee> <count>`",
                        path.display()
                    )
                })
        })
        .collect();
    let mut built: Vec<(&str, (String, String))> = Vec::new();
    for [outer, callee, count] in markers {
        let count: usize = count.parse().unwrap_or_else(|_| {
            panic!(
                "{}: `{O2_CALLS} {outer} {callee} {count}` count is not a number",
                path.display()
            )
        });
        if !built.iter().any(|(caller, _)| *caller == outer) {
            built.push((outer, o2_ir(path, outer)));
        }
        let (_, (before, optimized)) = built
            .iter()
            .find(|(caller, _)| *caller == outer)
            .expect("the caller was just built");
        for function in [outer, callee] {
            assert!(
                function_text(before, function).is_some(),
                "{}: no function named `{function}`",
                path.display()
            );
        }
        let header = function_text(before, outer)
            .and_then(|text| text.lines().next())
            .expect("the caller is in the IR");
        assert!(
            header.contains(&format!("@sisu.{outer}()")),
            "{}: o2-calls caller `{outer}` takes parameters; its call sites can decide the \
             call, so mark a caller without parameters",
            path.display()
        );
        let body = function_text(optimized, outer)
            .unwrap_or_else(|| panic!("{}: no `{outer}` in the -O2 IR", path.display()));
        let target = format!("@sisu.{callee}(");
        let kept = body
            .lines()
            .filter(|line| line.contains("call ") && line.contains(&target))
            .count();
        assert_eq!(
            kept,
            count,
            "{}: -O2 keeps {kept} calls to `{callee}` in `{outer}`, not {count}; make `{callee}` \
             recurse, and not to a depth the call sites fix, so the optimizer keeps it\n{body}",
            path.display()
        );
    }
}

/// `o0`, which must equal `o2`, as text; `what` names the stream in the failure.
fn agreed(path: &Path, what: &str, o0: &[u8], o2: &[u8]) -> String {
    let (o0, o2) = (String::from_utf8_lossy(o0), String::from_utf8_lossy(o2));
    assert!(
        o0 == o2,
        "{}: O0 and -O2 disagree on {what}\nO0:\n{o0}\n-O2:\n{o2}",
        path.display()
    );
    o0.into_owned()
}

/// Checks that each `(path, actual)` file holds `actual`. With `bless`, writes each `actual` to
/// its file instead, but only if every file exists: bless never creates an expectation file, and
/// writes all of them or none.
fn expect_files(files: &[(PathBuf, String)], bless: bool) {
    if bless {
        for (path, _) in files {
            assert!(
                path.exists(),
                "{} is missing; create it empty, then bless again",
                path.display()
            );
        }
        for (path, actual) in files {
            fs::write(path, actual).expect("writes the expectation file");
        }
    } else {
        for (path, actual) in files {
            let display = path.display();
            let expected = fs::read_to_string(path).unwrap_or_else(|error| {
                panic!("{display}: {error}; create it empty, then run with SISU_BLESS=1")
            });
            assert_eq!(
                *actual, expected,
                "{display} does not match; run with SISU_BLESS=1 to rewrite it"
            );
        }
    }
}
