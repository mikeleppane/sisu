//! The unused-value warning: which expression statements and block tails drop a value.

use super::*;

/// Line 10 of `in_body(body)` is `body`, indented four spaces; a `unit` call follows it.
fn in_body(body: &str) -> String {
    format!(
        "fn f() -> i64 {{ 1 }}
fn g() {{}}
fn h() {{}}
class A {{
    fn k(self) -> i64 {{ 1 }}
    fn m(self) {{}}
}}
fn main() {{}}
fn t(c: bool, d: bool, n: i64, o: i64?, xs: [i64], a: A?) {{
    {body}
    g()
}}"
    )
}

/// The diagnostics of `src` are exactly unused-value warnings at `expected`: (line, col, the
/// text the warning covers).
fn warns(src: &str, expected: &[(usize, usize, &str)]) {
    let ds = diags(src);
    let found: Vec<_> = ds
        .iter()
        .map(|d| {
            let span = d.span();
            let (line, col) = at(src, span);
            (d.code(), line, col, &src[span.range()])
        })
        .collect();
    let expected: Vec<_> = expected
        .iter()
        .map(|&(line, col, text)| (Code::UnusedValue, line, col, text))
        .collect();
    assert_eq!(found, expected, "{src}");
}

#[test]
fn unused_value_warns() {
    warns(
        "fn main() {\n    let n = 1\n    u8(n)\n}",
        &[(3, 5, "u8(n)")],
    );
    warns("fn g() { 1 }\nfn main() {}", &[(1, 10, "1")]);
    let rows: &[(&str, &[(usize, &str)])] = &[
        ("f()", &[(5, "f()")]),
        ("if c { 1 } else { 2 }", &[(12, "1"), (23, "2")]),
        // The checker accepts an `if` with no `else` whose branch has a value.
        ("if c { f() }", &[(12, "f()")]),
        ("if let v = o { v } else { 0 }", &[(20, "v"), (31, "0")]),
        // The inner `if` joins with `None` to `i64?`, so its value is wrapped.
        (
            "if c { if d { 1 } else { 2 } } else { None }",
            &[(19, "1"), (30, "2"), (43, "None")],
        ),
        ("while c { f() }", &[(15, "f()")]),
        ("while let v = o { v }", &[(23, "v")]),
        ("for x in xs { x }", &[(19, "x")]),
        // A block is not the value either: its tail is.
        ("({ 1 })", &[(8, "1")]),
        // A `?.` call lowers to an `if let`, whose value is the call's, as an optional.
        ("a?.k()", &[(5, "a?.k()")]),
        ("\"a\" + \"b\"", &[(5, "\"a\" + \"b\"")]),
    ];
    for (body, expected) in rows {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(col, text)| (10, col, text))
            .collect();
        warns(&in_body(body), &expected);
    }
}

#[test]
fn unused_value_clean() {
    for body in [
        "let _ = u8(n)",
        "a?.m()",
        "if c { g() } else { h() }",
        "if let v = o { g() }",
        "while c { g() }",
        "while let v = o { g() }",
        "for x in xs { g() }",
        // `never` in a branch tail that nothing uses.
        "if c { panic(\"x\") }",
        "let y = if c { 1 } else { 2 }",
    ] {
        clean(&in_body(body));
    }
    clean("fn f() -> i64 { 1 }\nfn main() {}");
    clean("fn main() {\n    panic(\"x\")\n}");
    // A `panic` that is not last makes what follows unreachable, but is no unused value.
    codes(
        "fn main() {\n    panic(\"x\")\n    print(1)\n}",
        &[(Code::UnreachableCode, (3, 5))],
    );
}

#[test]
fn unused_value_skips_errors() {
    codes(
        "fn main() {\n    undefined_name\n}",
        &[(Code::BindingNotFound, (2, 5))],
    );
    codes(
        "fn main() {\n    undefined_name\n    print(1)\n}",
        &[(Code::BindingNotFound, (2, 5))],
    );
}

#[test]
fn unused_value_fix() {
    let src = "fn g() { 1 }\nfn main() {}";
    let d = diagnostic(src, Code::UnusedValue, (1, 10, "1"), &[]);
    assert_eq!(
        fixes(src, &d),
        [(false, vec![(1, 10, "", "let _ = ".into())])]
    );
    let fixed = applied(src, &d);
    assert_eq!(fixed, "fn g() { let _ = 1 }\nfn main() {}");
    clean(&fixed);
}
