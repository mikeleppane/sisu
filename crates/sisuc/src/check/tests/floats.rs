//! `f64`: literals typed by context, negation, exactness, no mixing with integers, and `f64(n)`.

use super::*;

/// A function `f` over the scalar types, whose body is `let _ = <expr>` on line 2, so `expr`
/// starts at column 13.
fn scalars(expr: &str) -> String {
    format!(
        "fn f(n: i64, x: f64, b: u8, c: char, s: string) {{\n    let _ = {expr}\n}}\nfn main() {{}}"
    )
}

#[test]
fn literal_typing() {
    let rows = [
        ("fn main() { let x: f64 = 1 }", "(let x#0 1.0)"),
        ("fn main() { let x: f64? = 1 }", "(let x#0 (wrap 1.0))"),
        ("fn main() { let x: f64 = 1 + 2 }", "(let x#0 (+ 1.0 2.0))"),
        (
            "fn main() {\n    let x = 2.5\n    let _ = 2 * x\n}",
            "(* 2.0 x#0)",
        ),
        (
            "fn main() {\n    let x = 2.5\n    let _ = x * 2\n}",
            "(* x#0 2.0)",
        ),
        ("fn main() { let _ = 1 + 2.5 }", "(+ 1.0 2.5)"),
        // With no `f64` beside it, an integer literal stays an `i64`.
        ("fn main() { let n = 1 }", "(let n#0 1)"),
    ];
    for (src, part) in rows {
        contains(src, &[part]);
    }
}

#[test]
fn negation_type() {
    // `-x` is an `f64`, so it fits an `f64` and not an `i64`.
    contains(
        "fn main() {\n    let x = 2.5\n    let y = -x\n    let w: f64 = y\n}",
        &["(let y#1 (- x#0))"],
    );
    codes(
        "fn main() {\n    let x = 2.5\n    let y: i64 = -x\n}",
        &[(Code::Mismatch, (3, 18))],
    );
    contains("fn main() { let z: f64 = -1 }", &["(let z#0 -1.0)"]);
    // An `f64?` is held to `f64`, the type it would negate as: see
    // tests/diagnostics/E0032-negate-optional.
    diagnostic(
        "fn f(x: f64?) {\n    let _ = -x\n}\nfn main() {}",
        Code::Mismatch,
        (2, 14, "x"),
        &[],
    );
}

#[test]
fn float_never_integer() {
    let src = "fn main() {\n    let n: i64 = 1.5\n}";
    let d = diagnostic(src, Code::Mismatch, (2, 18, "1.5"), &[]);
    assert_eq!(fixes(src, &d), []);
}

#[test]
fn exactness() {
    let rows = [
        ("9007199254740993", Some("9007199254740992.0")),
        ("9223372036854775807", Some("9.223372036854776e18")),
        // Exactly -2^63.
        ("-9223372036854775808", None),
        // 2^53, the last integer before the gaps.
        ("9007199254740992", None),
    ];
    for (literal, nearest) in rows {
        let src = format!("fn main() {{\n    let x: f64 = {literal}\n}}");
        match nearest {
            Some(nearest) => {
                let d = diagnostic(&src, Code::NotExactlyF64, (2, 18, literal), &[]);
                assert_eq!(
                    fixes(&src, &d),
                    [(true, vec![(2, 18, literal, nearest.to_string())])]
                );
                fix_removes_its_code(&src, &d);
            }
            None => clean(&src),
        }
    }
}

#[test]
fn mixing() {
    let rows = [
        ("n * x", (13, "n"), "f64(n) * x"),
        ("x * n", (17, "n"), "x * f64(n)"),
        ("n < x", (13, "n"), "f64(n) < x"),
        ("n == x", (13, "n"), "f64(n) == x"),
        ("x != n", (18, "n"), "x != f64(n)"),
        ("b * x", (13, "b"), "f64(i64(b)) * x"),
    ];
    for (expr, (col, found), fixed) in rows {
        let src = scalars(expr);
        let d = diagnostic(&src, Code::Mismatch, (2, col, found), &[]);
        assert!(d.fixes().iter().all(|f| !f.certain()), "{src}");
        let fixed = scalars(fixed);
        assert_eq!(applied(&src, &d), fixed);
        clean(&fixed);
    }
    // The `string` drives `+`, as beside any other type, and is the secondary label.
    diagnostic(
        &scalars("x + \"a\""),
        Code::Mismatch,
        (2, 13, "x"),
        &[(2, 17)],
    );
}

#[test]
fn int_to_float() {
    // An `f64`, which the function returns.
    contains(
        "fn g(n: i64) -> f64 { f64(n) }\nfn main() {}",
        &["(f64 n#0)"],
    );
    let rows = [
        ("f64(b)", (17, "b"), Some("f64(i64(b))")),
        ("f64(c)", (17, "c"), Some("f64(i64(c))")),
        ("f64(s)", (17, "s"), None),
    ];
    for (expr, (col, found), fixed) in rows {
        let src = scalars(expr);
        let d = diagnostic(&src, Code::Mismatch, (2, col, found), &[]);
        match fixed {
            Some(fixed) => {
                let fixed = scalars(fixed);
                assert_eq!(applied(&src, &d), fixed);
                clean(&fixed);
            }
            None => assert_eq!(fixes(&src, &d), [], "{src}"),
        }
    }
    diagnostic(
        &scalars("f64(x)"),
        Code::AlreadyConverted,
        (2, 17, "x"),
        &[],
    );
}

/// Where the context wants an `i64`, the `f64` is the operand reported, with no fix: a compound
/// assignment's target and a declared type are not what changes. The `f64` is reported in
/// either order.
#[test]
fn mixing_under_an_integer() {
    let rows = [
        (
            "fn main() {\n    let mut n = 1\n    n += 1.5\n    print(n)\n}",
            (3, 10, "1.5"),
        ),
        (
            "fn f(x: f64) {\n    let mut n = 1\n    n *= x\n    print(n)\n}\nfn main() {}",
            (3, 10, "x"),
        ),
        (
            "fn f(n: i64, x: f64) -> i64 {\n    n * x\n}\nfn main() {}",
            (2, 9, "x"),
        ),
        (
            "fn f(n: i64, x: f64) -> i64 {\n    x * n\n}\nfn main() {}",
            (2, 5, "x"),
        ),
        (
            "fn f(n: i64, x: f64) -> i64 {\n    return n * x\n}\nfn main() {}",
            (2, 16, "x"),
        ),
        (
            "fn f(n: i64, x: f64) -> i64 {\n    return x * n\n}\nfn main() {}",
            (2, 12, "x"),
        ),
        (
            "fn f(n: i64, x: f64) {\n    let y: i64 = n * x\n    print(y)\n}\nfn main() {}",
            (2, 22, "x"),
        ),
        (
            "fn f(n: i64, x: f64) {\n    let y: i64 = x * n\n    print(y)\n}\nfn main() {}",
            (2, 18, "x"),
        ),
    ];
    for (src, at) in rows {
        let d = diagnostic(src, Code::Mismatch, at, &[]);
        assert_eq!(fixes(src, &d), [], "{src}");
    }
}
