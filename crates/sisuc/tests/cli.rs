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
        "class None is self . ? ?. ?? _",
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
        names[..9],
        [
            "Class",
            "NoneKw",
            "Is",
            "SelfKw",
            "Dot",
            "Question",
            "QuestionDot",
            "QuestionQuestion",
            "Underscore"
        ]
    );
}

#[test]
fn emit_tokens_reserved() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join("emit_tokens_reserved.sisu"), "match\n").expect("writes");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args(["--emit", "tokens", "emit_tokens_reserved.sisu"])
        .output()
        .expect("sisuc starts");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("1:1 Reserved match\n"));
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
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(at_severity(&stderr, "error") == Some(0), "{stderr}");
    assert!(
        stderr
            .contains("error[E0010]: unexpected character `&`\n --> lexer_error_exits_1.sisu:1:3"),
        "{stderr}"
    );
}

#[test]
fn bare_flag_is_a_usage_error() {
    assert_usage_errors(&[&["--check"]]);
}

/// Runs `sisuc` with each of `cases` in the target tmp dir, and checks that each is a usage
/// error.
fn assert_usage_errors(cases: &[&[&str]]) {
    for args in cases {
        let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
            .current_dir(env!("CARGO_TARGET_TMPDIR"))
            .args(*args)
            .output()
            .expect("sisuc starts");
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.starts_with("usage: sisuc"), "{args:?}: {stderr}");
    }
}

/// `--rc-checks` is accepted only right after an optional `-O2`, and only for a build.
#[test]
fn rc_checks_is_build_only() {
    let cases: [&[&str]; 8] = [
        &["--rc-checks", "--explain", "E0001"],
        &["--rc-checks", "--emit", "ir", "f.sisu"],
        &["-O2", "--rc-checks", "--emit", "ir", "f.sisu"],
        &["--rc-checks", "--check", "f.sisu"],
        &["--rc-checks", "-O2", "f.sisu", "out"],
        &["--emit", "ir", "--rc-checks"],
        &["--check", "--rc-checks"],
        &["--rc-checks", "--rc-checks", "f.sisu"],
    ];
    assert_usage_errors(&cases);
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
        "fn main() {\n    let mut x = 1\n    x += 2\n}\n",
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
        "(fn main () unit (block (let mut x 1) (+= x 2)))\n"
    );
}

#[test]
fn emit_ast_and_tir_print_mut() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(
        dir.join("emit_mut.sisu"),
        "class P {\n    name: i64\n    mut hp: i64\n}\nfn main() {\n    let mut x = 1\n    x = 2\n}\n",
    )
    .expect("writes");
    let emit = |what: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
            .current_dir(dir)
            .args(["--emit", what, "emit_mut.sisu"])
            .output()
            .expect("sisuc starts");
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert_eq!(
        emit("ast"),
        "(class P (name i64) (mut hp i64))\n(fn main () unit (block (let mut x 1) (= x 2)))\n"
    );
    assert_eq!(
        emit("tir"),
        "(class P (name i64) (mut hp i64))\n(fn main () unit (block (let mut x#0 1) (= x#0 2)))\n"
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
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(at_severity(&stderr, "error") == Some(0), "{stderr}");
    assert!(
        stderr.contains(
            "error[E0011]: expected `fn` or `class`, found `let`\n --> parse_error_exits_1.sisu:1:1"
        ),
        "{stderr}"
    );
}

/// The offset of the first diagnostic header of `severity` in `stderr`, with or without a code.
fn at_severity(stderr: &str, severity: &str) -> Option<usize> {
    [format!("{severity}["), format!("{severity}:")]
        .iter()
        .filter_map(|header| stderr.find(header.as_str()))
        .min()
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
        "fn main() {\n    let mut count = 0\n    print(count)\n}\n",
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("warning[W0002]: "));
}

#[test]
fn check_error_exits_1() {
    let out = check(
        "check_error_exits_1",
        "fn main() {\n    let n = 0\n    n = 1\n}\n",
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error[E0001]: cannot assign to `n`"));
}

#[test]
fn check_warning_and_error_exits_1() {
    let out = check(
        "check_warning_and_error_exits_1",
        "fn f() { let mut x = 0 }\nfn main() { print(missing) }\n",
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let warning = at_severity(&stderr, "warning").expect("a warning");
    let error = at_severity(&stderr, "error").expect("an error");
    assert!(warning < error, "{stderr}");
}

#[test]
fn explain_prints_the_explanation() {
    let expected = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/diagnostics/E0001.md"
    ))
    .expect("reads the explanation");
    for code in ["E0001", "e0001"] {
        let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
            .args(["--explain", code])
            .output()
            .expect("sisuc starts");
        assert_eq!(out.status.code(), Some(0), "{code}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected, "{code}");
        assert_eq!(String::from_utf8_lossy(&out.stderr), "", "{code}");
    }
}

#[test]
fn explain_rejects_an_unknown_code() {
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .args(["--explain", "E9999"])
        .output()
        .expect("sisuc starts");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "sisuc: unknown diagnostic code `E9999`\n"
    );
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

/// Runs `sisuc --check` on a file named `name` that holds `bytes`; returns the exit code and stderr.
fn check_bytes(name: &str, bytes: &[u8]) -> (Option<i32>, String) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join(name), bytes).expect("writes");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args(["--check", name])
        .output()
        .expect("sisuc starts");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into(),
    )
}

#[test]
fn invalid_utf8_on_line_two_is_a_diagnostic_at_its_line_and_column() {
    // Line 1 holds two non-ASCII chars on four bytes: the column counts chars, not bytes.
    let name = "invalid_utf8_line_two.sisu";
    let (code, stderr) = check_bytes(name, b"// \xc3\xa4\xc3\xa4\nfn main() {\xff}\n");
    assert_eq!(code, Some(1));
    assert!(
        stderr.starts_with(&format!(
            "error[E0072]: source is not valid UTF-8\n --> {name}:2:12\n"
        )),
        "{stderr}"
    );
}

#[test]
fn invalid_utf8_at_the_start_is_a_diagnostic() {
    let name = "invalid_utf8_start.sisu";
    let (code, stderr) = check_bytes(name, b"\xff\nfn main() {}\n");
    assert_eq!(code, Some(1));
    assert!(
        stderr.starts_with(&format!(
            "error[E0072]: source is not valid UTF-8\n --> {name}:1:1\n"
        )),
        "{stderr}"
    );
}

#[test]
fn a_truncated_character_at_end_of_file_is_a_diagnostic() {
    // `\xe2\x82` starts the three-byte `€` and stops.
    let name = "invalid_utf8_truncated.sisu";
    let (code, stderr) = check_bytes(name, b"fn main() {}\n\xe2\x82");
    assert_eq!(code, Some(1));
    assert!(
        stderr.starts_with(&format!(
            "error[E0072]: source is not valid UTF-8\n --> {name}:2:1\n"
        )),
        "{stderr}"
    );
}

#[test]
fn invalid_utf8_after_crlf_line_breaks_counts_each_break_as_one_line() {
    let (name, name_start) = ("invalid_utf8_crlf.sisu", "invalid_utf8_crlf_start.sisu");
    let (code, stderr) = check_bytes(name, b"a\r\nb\r\nxy\xff\r\n");
    assert_eq!(code, Some(1));
    assert!(
        stderr.starts_with(&format!(
            "error[E0072]: source is not valid UTF-8\n --> {name}:3:3\n"
        )),
        "{stderr}"
    );
    // The bad byte is the first of its line, right after the break.
    let (_, stderr) = check_bytes(name_start, b"a\r\n\xff\r\n");
    assert!(
        stderr.starts_with(&format!(
            "error[E0072]: source is not valid UTF-8\n --> {name_start}:2:1\n"
        )),
        "{stderr}"
    );
}

#[test]
fn an_encoded_surrogate_is_invalid_utf8_at_its_first_byte() {
    // `\xed\xa0\x80` would be U+D800, which UTF-8 does not allow.
    let name = "invalid_utf8_surrogate.sisu";
    let (code, stderr) = check_bytes(name, b"a\nxy\xed\xa0\x80\n");
    assert_eq!(code, Some(1));
    assert!(
        stderr.starts_with(&format!(
            "error[E0072]: source is not valid UTF-8\n --> {name}:2:3\n"
        )),
        "{stderr}"
    );
}

#[test]
fn mem2reg_removes_every_alloca() {
    // `loops.sisu` declares `let mut sq` inside the loop body: its `alloca` must still sit in the
    // entry block, or `mem2reg` leaves it in place.
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/run"))
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
        assert!(
            section.contains("define internal void @sisu.main()"),
            "{section}"
        );
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
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error[E0001]: cannot assign to `n`"));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

#[test]
fn emit_ir_warning_still_prints_ir() {
    let out = run_on(
        "emit_ir_warning_still_prints_ir",
        "fn main() {\n    let mut count = 0\n    print(count)\n}\n",
        &["--emit", "ir"],
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .starts_with("warning[W0002]: `count` is never reassigned")
    );
    ir_sections(&String::from_utf8_lossy(&out.stdout));
}

#[test]
fn emit_tir() {
    let out = run_on(
        "emit_tir",
        "fn main() {\n    let mut i = 0\n    while i < 2 {\n        i = i + 1\n    }\n}\n",
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
        "(fn main () unit (block (let mut i#0 0) (loop (block (if (< i#0 2) (block (= i#0 (+ i#0 1))) (block (break)))))))\n"
    );
}

#[test]
fn emit_tir_shows_moves() {
    let out = run_on(
        "emit_tir_shows_moves",
        "fn f(xs: [i64]) -> [i64] { xs }\nfn main() {}\n",
        &["--emit", "tir"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("(move xs#0)"));
}

#[test]
fn emit_with_an_error_prints_nothing_on_stdout() {
    let flags: [(&[&str], &str); 2] = [
        (&["--emit", "tir"], "error[E0001]: "),
        (
            &["--format", "json", "--emit", "tir"],
            r#"{"version":1,"kind":"diagnostic","code":"E0001","#,
        ),
    ];
    for (flags, first) in flags {
        let out = run_on(
            "emit_with_an_error_prints_nothing_on_stdout",
            "fn main() {\n    let n = 0\n    n = 1\n}\n",
            flags,
        );
        assert_eq!(out.status.code(), Some(1), "{flags:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.starts_with(first), "{flags:?}: {stderr}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "", "{flags:?}");
    }
}

/// On a valid file, `--format json` leaves stdout as text has it, and stderr holds only the
/// summary.
#[test]
fn json_emit_keeps_stdout() {
    for stage in ["tokens", "ast", "tir", "ir", "ir-raw"] {
        let text = run_on("json_emit_keeps_stdout", SQUARE, &["--emit", stage]);
        let json = run_on(
            "json_emit_keeps_stdout",
            SQUARE,
            &["--format", "json", "--emit", stage],
        );
        for out in [&text, &json] {
            assert_eq!(
                out.status.code(),
                Some(0),
                "{stage}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        assert!(!text.stdout.is_empty(), "{stage}");
        assert_eq!(json.stdout, text.stdout, "{stage}");
        assert_eq!(String::from_utf8_lossy(&text.stderr), "", "{stage}");
        assert_eq!(
            String::from_utf8_lossy(&json.stderr),
            format!("{CLEAN_SUMMARY}\n"),
            "{stage}"
        );
    }
}

const FIB: &str = "fn fib(n: i64) -> i64 {\n    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }\n}\n\nfn main() {\n    print(fib(30))\n}\n";

#[test]
fn o2_combinations_are_usage_errors() {
    let cases: [&[&str]; 14] = [
        &["-O2", "--explain", "E0001"],
        &["--explain", "E0001", "-O2"],
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
    assert_usage_errors(&cases);
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
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/run");
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
    // The passes see the target's triple and data layout, as the object file does.
    for wanted in ["\ntarget datalayout = \"", "\ntarget triple = \""] {
        assert!(after.contains(wanted), "missing {wanted:?} in\n{after}");
    }
}

/// A `print` that may never run does not make the loop reload the array's length: the
/// runtime's print functions write only memory the module cannot reach, so `-O2` proves
/// every `xs[i]` in range from the loop bound and drops both bounds checks.
#[test]
fn print_in_a_loop_keeps_no_bounds_check_at_o2() {
    let source = "fn main() {\n    let xs = [1; 10_000]\n    let mut total = 0\n\
                  for round in 0..1000 {\n        for i in 0..10_000 {\n\
                  if xs[i] == 0 { print(i) }\n            total += xs[i]\n        }\n    }\n\
                  print(total)\n}\n";
    let out = run_on("print_in_a_loop", source, &["-O2", "--emit", "ir"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let (before, after) = stdout
        .split_once("\n; after default<O2>\n")
        .unwrap_or_else(|| panic!("{stdout}"));
    assert!(before.contains("call void @sisu_panic_bounds("), "{before}");
    assert!(!after.contains("call void @sisu_panic_bounds("), "{after}");
}

/// The JSON summary line of a run with no diagnostics.
const CLEAN_SUMMARY: &str =
    r#"{"version":1,"kind":"summary","errors":0,"warnings":0,"explanations":{}}"#;

#[test]
fn json_unreadable_input_is_a_tool_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(env!("CARGO_TARGET_TMPDIR"))
        .args(["--format", "json", "--check", "no_such_dir/missing.sisu"])
        .output()
        .expect("sisuc starts");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .collect::<Vec<_>>(),
        [
            r#"{"version":1,"kind":"tool_error","message":"cannot read no_such_dir/missing.sisu: No such file or directory (os error 2)"}"#,
            CLEAN_SUMMARY
        ]
    );
}

/// `--format` is accepted only as the first two arguments, with `text` or `json`, and never with
/// `--explain`. A usage error stays text, also after `--format json`.
#[test]
fn format_usage_errors() {
    assert_usage_errors(&[
        &["--format", "json"],
        &["--format", "yaml", "--check", "a.sisu"],
        &["--format", "text", "--explain", "E0001"],
        // A later `--format` would otherwise fill the input slot of a valid shape.
        &["--check", "--format"],
        &["--emit", "tir", "--format"],
        &["-O2", "--emit", "ir", "--format"],
        &["--explain", "--format"],
    ]);
}

#[test]
fn json_link_failure_stays_json() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join("json_link_failure.sisu"), "fn main() {}\n").expect("writes");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args([
            "--format",
            "json",
            "json_link_failure.sisu",
            "missing-dir/out",
        ])
        .output()
        .expect("sisuc starts");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert!(
        lines
            .iter()
            .all(|l| l.starts_with(r#"{"version":1,"kind":""#)),
        "{stderr}"
    );
    assert_eq!(lines.last(), Some(&CLEAN_SUMMARY), "{stderr}");
    let messages: Vec<String> = lines
        .iter()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("each line is JSON"))
        .filter(|v| v["kind"] == "tool_error")
        .map(|v| v["message"].as_str().expect("a string message").to_string())
        .collect();
    let [message] = messages.as_slice() else {
        panic!("one tool error in {stderr}");
    };
    // cc's own stderr follows the status, and names the output it could not write.
    let (status, detail) = message
        .split_once('\n')
        .expect("cc's output after a newline");
    assert!(status.starts_with("cc failed: "), "{message}");
    assert!(detail.contains("missing-dir/out"), "{message}");
    assert_eq!(detail, detail.trim(), "{message}");
}

#[test]
fn json_overwrite_is_a_tool_error() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join("json_overwrite.sisu"), "fn main() {}\n").expect("writes");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args([
            "--format",
            "json",
            "json_overwrite.sisu",
            "json_overwrite.sisu",
        ])
        .output()
        .expect("sisuc starts");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .collect::<Vec<_>>(),
        [
            r#"{"version":1,"kind":"tool_error","message":"output json_overwrite.sisu would overwrite the input"}"#,
            CLEAN_SUMMARY
        ]
    );
    assert_eq!(
        fs::read_to_string(dir.join("json_overwrite.sisu")).expect("reads"),
        "fn main() {}\n"
    );
}

#[test]
fn warnings_only_build_succeeds_with_summary() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(
        dir.join("warnings_only_build.sisu"),
        "fn main() {\n    let mut count = 3\n    print(count)\n}\n",
    )
    .expect("writes");
    let exe = dir.join("warnings_only_build");
    let out = Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .arg("warnings_only_build.sisu")
        .arg(&exe)
        .output()
        .expect("sisuc starts");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(
        stderr.ends_with(
            "\n\nsisuc: 1 warning\nsisuc: for more on a code, run `sisuc --explain <code>`: W0002\n"
        ),
        "{stderr}"
    );
    let run = Command::new(&exe).output().expect("the program starts");
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n");
}

#[test]
fn format_text_is_the_default() {
    let source = "fn main() {\n    let n = 0\n    n = 1\n}\n";
    let plain = run_on("format_text_is_the_default", source, &["--check"]);
    let text = run_on(
        "format_text_is_the_default",
        source,
        &["--format", "text", "--check"],
    );
    assert_eq!(text.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&text.stderr).starts_with("error[E0001]: "),
        "{}",
        String::from_utf8_lossy(&text.stderr)
    );
    assert_eq!(text.stderr, plain.stderr);
}
