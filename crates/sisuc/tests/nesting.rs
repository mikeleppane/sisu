//! The parser bounds how deep a program nests (`MAX_DEPTH` in `parser.rs`), so no later stage
//! overflows its stack on a deep tree.
//!
//! Each construct below has a generator for a program that nests it `n` times and the byte offset
//! of the token where the parser runs out of depth. `limit` is the deepest `n` that compiles: the
//! depth counts the function body and the statement around the construct too, and an `if` or a
//! `while` adds a block, so `limit` differs between constructs. The programs are too large to
//! keep as files.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// Classes and functions that the generated programs call.
const PRELUDE: &str = "\
class N {
    next: N?
    v: i64

    fn me(self) -> N {
        self
    }
}

fn id(x: i64) -> i64 {
    x
}

fn some(x: i64) -> i64? {
    x
}

";

/// A generated program and the byte offset of the token where the depth runs out.
struct Generated {
    source: String,
    fail: usize,
}

/// `PRELUDE`, `fn main() {`, then `lines`, then `}`. `fail` is a byte offset into `lines`.
fn in_main(lines: &str, fail: usize) -> Generated {
    let head = format!("{PRELUDE}fn main() {{\n");
    Generated {
        fail: head.len() + fail,
        source: format!("{head}{lines}}}\n"),
    }
}

/// The line `    <before><open × n><leaf><close × n><after>`.
fn wrapped(before: &str, open: &str, leaf: &str, close: &str, after: &str, n: usize) -> String {
    format!(
        "    {before}{}{leaf}{}{after}\n",
        open.repeat(n),
        close.repeat(n)
    )
}

/// `print(` nests `n` times, then `1`.
fn paren(n: usize) -> Generated {
    in_main(&wrapped("print(", "(", "1", ")", ")", n), 10 + n)
}

fn call(n: usize) -> Generated {
    in_main(&wrapped("print(", "id(", "1", ")", ")", n), 10 + 3 * n)
}

fn array(n: usize) -> Generated {
    in_main(&wrapped("let a = ", "[", "1", "]", "", n), 12 + n)
}

/// The last `-`: a unary operator fails at itself, before it is consumed.
fn neg(n: usize) -> Generated {
    in_main(&wrapped("print(", "-", "1", "", ")", n), 10 + n - 1)
}

fn not(n: usize) -> Generated {
    in_main(&wrapped("print(", "!", "true", "", ")", n), 10 + n - 1)
}

/// The `n`th `+`: a left-associative chain is as deep as it is long.
fn plus(n: usize) -> Generated {
    let line = format!("    print(1{})\n", "+1".repeat(n));
    in_main(&line, 11 + 2 * (n - 1))
}

fn and(n: usize) -> Generated {
    let line = format!("    print(true{})\n", "&&true".repeat(n));
    in_main(&line, 14 + 6 * (n - 1))
}

/// The `n`th `??`, which is right-associative.
fn coalesce(n: usize) -> Generated {
    let line = format!("    let x: i64? = None\n    print({}1)\n", "x??".repeat(n));
    in_main(
        &line,
        "    let x: i64? = None\n".len() + 10 + 1 + 3 * (n - 1),
    )
}

/// The `0` inside the `n`th `[`.
fn index(n: usize) -> Generated {
    let first = wrapped("let a = ", "[", "1", "]", "", n);
    let line = format!("{first}    let b = a{}\n", "[0]".repeat(n));
    in_main(&line, first.len() + 13 + 3 * (n - 1) + 1)
}

/// The `n`th `.`.
fn method_chain(n: usize) -> Generated {
    let first = "    let n = N(next: None, v: 1)\n";
    let line = format!("{first}    let m = n{}\n", ".me()".repeat(n));
    in_main(&line, first.len() + 13 + 5 * (n - 1))
}

/// The `n`th `?.`.
fn safe_chain(n: usize) -> Generated {
    let first = "    let n: N? = None\n";
    let line = format!("{first}    let m = n{}\n", "?.next".repeat(n));
    in_main(&line, first.len() + 13 + 6 * (n - 1))
}

/// `head {` nested `n` times around `print(1)`; the `1` is where a block runs out of depth.
fn blocks(head: &str, n: usize) -> Generated {
    let open = format!("{head} {{\n");
    let line = format!("{}print(1)\n{}", open.repeat(n), "}\n".repeat(n));
    in_main(&line, open.len() * n + 6)
}

fn nested_if(n: usize) -> Generated {
    blocks("if true", n)
}

fn nested_if_let(n: usize) -> Generated {
    blocks("if let y = some(1)", n)
}

fn nested_while(n: usize) -> Generated {
    blocks("while false", n)
}

fn nested_for(n: usize) -> Generated {
    blocks("for i in 0..1", n)
}

/// An `else if` chain; the `==` of the last condition.
fn else_if(n: usize) -> Generated {
    let first = "    let x = 1\n    if x == 0 {\n";
    let clause = "    } else if x == 0 {\n";
    let body = format!("{first}{}    }}\n", clause.repeat(n));
    in_main(&body, first.len() + clause.len() * (n - 1) + 16)
}

/// The element type `n` brackets deep; fails at its name.
fn array_type(n: usize) -> Generated {
    in_main(&wrapped("let a: ", "[", "i64", "]", " = []", n), 11 + n)
}

/// Where the byte `offset` of `source` is, as 1-based `(line, column)`.
fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.len() - before.rfind('\n').map_or(0, |i| i + 1) + 1;
    (line, col)
}

fn sisuc(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sisuc"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("sisuc starts")
}

/// Builds and runs the program `n` deep: it must compile and exit 0 without a diagnostic.
fn compiles_at_the_limit(name: &str, program: fn(usize) -> Generated, limit: usize) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join(format!("{name}.sisu")), program(limit).source).expect("writes");
    let exe = format!("{name}.out");
    let build = sisuc(dir, &[&format!("{name}.sisu"), &exe]);
    assert_eq!(
        (build.status.code(), String::from_utf8_lossy(&build.stderr)),
        (Some(0), "".into())
    );
    let run = Command::new(dir.join(&exe))
        .output()
        .expect("program starts");
    assert_eq!(run.status.code(), Some(0));
}

/// One level past the limit: `--check` exits 1 with the one message, at the failing token.
fn errors_past_the_limit(name: &str, program: fn(usize) -> Generated, limit: usize) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let Generated { source, fail } = program(limit + 1);
    fs::write(dir.join(format!("{name}.sisu")), &source).expect("writes");
    let out = sisuc(dir, &["--check", &format!("{name}.sisu")]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let (line, col) = line_col(&source, fail);
    let mut lines = stderr.lines();
    assert_eq!(lines.next(), Some("error: nesting is too deep"), "{name}");
    assert_eq!(
        lines.next().map(str::trim_start),
        Some(format!("--> {name}.sisu:{line}:{col}").as_str()),
        "{name}"
    );
    assert!(
        stderr.contains("= help: the limit is 256 levels"),
        "{stderr}"
    );
    assert_eq!(stderr.matches("error:").count(), 1, "one error: {name}");
}

macro_rules! construct {
    ($name:ident, $program:ident, $limit:expr) => {
        mod $name {
            #[test]
            fn compiles_at_the_limit() {
                super::compiles_at_the_limit(
                    concat!("nesting_", stringify!($name)),
                    super::$program,
                    $limit,
                );
            }

            #[test]
            fn errors_one_level_past_the_limit() {
                super::errors_past_the_limit(
                    concat!("nesting_past_", stringify!($name)),
                    super::$program,
                    $limit,
                );
            }
        }
    };
}

construct!(parentheses, paren, 253);
construct!(calls, call, 253);
construct!(array_literals, array, 254);
construct!(unary_minus, neg, 253);
construct!(unary_not, not, 253);
construct!(sum_chain, plus, 253);
construct!(and_chain, and, 253);
construct!(coalesce_chain, coalesce, 253);
construct!(index_chain, index, 253);
construct!(method_chains, method_chain, 254);
construct!(safe_field_chain, safe_chain, 254);
construct!(ifs, nested_if, 126);
construct!(if_lets, nested_if_let, 126);
construct!(else_if_chain, else_if, 252);
construct!(whiles, nested_while, 253);
construct!(fors, nested_for, 253);
construct!(array_types, array_type, 254);

/// `--check` and `--emit ast` on `source` both exit 1 with the one message, however large it is:
/// the parser stops at the first level past the limit and builds nothing deeper.
fn assert_too_deep_both_ways(name: &str, source: &str) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join(format!("{name}.sisu")), source).expect("writes");
    for flags in [&["--check"][..], &["--emit", "ast"]] {
        let out = sisuc(dir, &[flags, &[&format!("{name}.sisu")]].concat());
        assert_eq!(out.status.code(), Some(1), "{name} {flags:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.starts_with("error: nesting is too deep\n"),
            "{name} {flags:?}: {stderr}"
        );
        assert_eq!(stderr.matches("error:").count(), 1, "{name} {flags:?}");
    }
}

#[test]
fn a_hundred_thousand_levels_is_an_error_not_an_abort() {
    let n = 100_000;
    let body = |expr: String| format!("fn main() {{\n    print({expr})\n}}\n");
    assert_too_deep_both_ways(
        "nesting_huge_parentheses",
        &body(format!("{}1{}", "(".repeat(n), ")".repeat(n))),
    );
    assert_too_deep_both_ways("nesting_huge_sum", &body(format!("1{}", "+1".repeat(n))));
    assert_too_deep_both_ways(
        "nesting_huge_else_if",
        &format!(
            "fn main() {{\n    let x = 1\n    if x == 0 {{\n    }}{} else {{\n    }}\n}}\n",
            " else if x == 0 {\n    }".repeat(n)
        ),
    );
    assert_too_deep_both_ways(
        "nesting_huge_type",
        &format!(
            "fn main() {{\n    let a: {}i64{} = []\n}}\n",
            "[".repeat(n),
            "]".repeat(n)
        ),
    );
}

/// Depth is the nesting at a point, not a count of everything parsed: a function of many
/// statements, each nesting a little, stays far below the limit at every statement.
#[test]
fn siblings_do_not_add_up_to_the_limit() {
    let group = |i: usize| {
        format!(
            "\
    print((1))
    print(-1 + 2 * 3 - 4)
    print(id(1) == 1)
    let a{i} = [1, 2]
    print(a{i}[0])
    let t{i}: [[i64]] = []
    if true {{
        print(1)
    }} else if false {{
        print(2)
    }}
    while false {{
    }}
    for i{i} in 0..1 {{
    }}
    print(some(1) ?? 2)
    print(1 < 2)
"
        )
    };
    let body: String = (0..300).map(group).collect();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let source = format!("{PRELUDE}fn main() {{\n{body}}}\n");
    fs::write(dir.join("nesting_siblings.sisu"), source).expect("writes");
    let out = sisuc(dir, &["--check", "nesting_siblings.sisu"]);
    assert_eq!(
        (out.status.code(), String::from_utf8_lossy(&out.stderr)),
        (Some(0), "".into())
    );
}
