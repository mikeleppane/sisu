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
    // `-x` with `x: f64?` names `f64` as the type it wants: only its fixture,
    // tests/diagnostics/E0032-negate-optional, can see that.
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

/// The `i64` the context wants reaches an operator inside the first operand, so the `f64` is
/// reported however deep it sits. A `+` that a `string` makes a join is reported as before.
#[test]
fn mixing_under_an_integer_nested() {
    let function = |body: &str| {
        format!("fn f(x: f64, n: i64, s: string) -> i64 {{\n    {body}\n}}\nfn main() {{}}")
    };
    let rows = [
        ("x * n * n", (2, 5, "x")),
        ("n * x * n", (2, 9, "x")),
        ("n * x + n", (2, 9, "x")),
        ("x * n + n", (2, 5, "x")),
        ("(n + n * x) - n", (2, 14, "x")),
    ];
    for (body, at) in rows {
        let src = function(body);
        let d = diagnostic(&src, Code::Mismatch, at, &[]);
        assert_eq!(fixes(&src, &d), [], "{src}");
    }
    // An `i64?` wants an `i64` too.
    diagnostic(
        "fn f(x: f64, n: i64) {\n    let y: i64? = x * n * n\n    print(y ?? 0)\n}\nfn main() {}",
        Code::Mismatch,
        (2, 19, "x"),
        &[],
    );
    // A `string` beside an `i64` is reported against `string`, with the guess to interpolate.
    for (body, at, other) in [
        ("s + n + n", (2, 9, "n"), (2, 5)),
        ("n + s + n", (2, 5, "n"), (2, 9)),
    ] {
        let src = function(body);
        let d = diagnostic(&src, Code::Mismatch, at, &[other]);
        assert_eq!(fixes(&src, &d).len(), 1, "{src}");
    }
}

/// `i64(x)` can fail, so it gives an `i64?`. `u8(x)` and `char(x)` take only an `i64`, and get
/// no fix: no one conversion turns an `f64` into one.
#[test]
fn float_conversions() {
    clean("fn f(x: f64) -> i64? { i64(x) }\nfn main() {}");
    clean("fn f(x: f64) -> i64 { i64(x) ?? 0 }\nfn main() {}");
    // The plain `i64` is a mismatch at the call; see tests/diagnostics/E0032-float-to-int.
    let src = "fn f(x: f64) -> i64 { i64(x) }\nfn main() {}";
    let d = diagnostic(src, Code::Mismatch, (1, 23, "i64(x)"), &[]);
    assert_eq!(fixes(src, &d), [], "{src}");
    for (expr, col) in [("u8(x)", 16), ("char(x)", 18)] {
        let src = scalars(expr);
        let d = diagnostic(&src, Code::Mismatch, (2, col, "x"), &[]);
        assert_eq!(fixes(&src, &d), [], "{src}");
    }
}

/// `f64` constants fold with Rust's `f64`, which never fails, and their uses lower to the
/// folded literal. `i64(x)` from an `f64` can fail, so it is an error in a constant; its fix
/// writes the whole number when there is one.
#[test]
fn float_constants() {
    let rows = [
        // A float literal beside an integer one makes both `f64`.
        ("const C = 2 * 3.0", "6.0"),
        ("const C: f64 = 1", "1.0"),
        ("const C = -2.5", "-2.5"),
        ("const C = 0.1 + 0.2", "0.30000000000000004"),
        ("const C = -7.5 % 2.0", "-1.5"),
        ("const C = 1.0 / 0.0", "inf"),
        ("const C = 0.0 / 0.0", "nan"),
        ("const C = 7.0 - 0.5 / 2.0", "6.75"),
        ("const N = 3\nconst C = f64(N)", "3.0"),
        // Negation flips the sign, so it is not `0.0 - x`: `-(0.0)` is `-0.0`.
        ("const C = -(1.0 + 1.5)", "-2.5"),
        ("const C = -(1.0 - 1.0)", "-0.0"),
        // Comparisons are IEEE: `nan` compares false, `-0.0` equals `0.0`.
        ("const C = 0.0 / 0.0 < 1.0", "false"),
        ("const C = 0.0 / 0.0 == 0.0 / 0.0", "false"),
        ("const C = 0.0 / 0.0 != 0.0 / 0.0", "true"),
        ("const C = -0.0 == 0.0", "true"),
        ("const C = 1.0 < 2.0 <= 2.0", "true"),
        ("const C = 2.0 > 1.0 >= 1.5", "false"),
        ("const C = 1.5 > 1.0", "true"),
    ];
    for (constant, value) in rows {
        let src = format!("{constant}\nfn main() {{\n    let x = C\n    print(x)\n}}");
        contains(&src, &[&format!("(let x#0 {value})")]);
    }
    // An integer that overflows inside an `f64` value still panics, on either side of an operator.
    for (src, column) in [
        ("const C = f64(9223372036854775807 + 1)", 15),
        ("const C = f64(9223372036854775807 + 1) * 2.0", 15),
        ("const C = 2.0 * f64(9223372036854775807 + 1)", 21),
    ] {
        codes(
            &format!("{src}\nfn main() {{}}"),
            &[(Code::ConstantPanics, (1, column))],
        );
    }
    // A declared type other than `f64` still holds an `f64` value to it.
    codes(
        "const C: i64 = 1.5\nfn main() {}",
        &[(Code::Mismatch, (1, 16))],
    );
    let error = (Code::ConversionInConstant, (1, 11));
    for (src, fixed) in [
        ("const N = i64(2.5)", None),
        ("const N = i64(2.0)", Some("const N: i64 = 2")),
        ("const N = i64(-0.0)", Some("const N: i64 = 0")),
        (
            "const N = i64(-9223372036854775808.0)",
            Some("const N: i64 = -9223372036854775808"),
        ),
        ("const N = i64(9223372036854775808.0)", None),
        ("const N = i64(1e300)", None),
        ("const N = i64(0.0 / 0.0)", None),
    ] {
        let src = format!("{src}\nfn main() {{}}");
        codes(&src, &[error]);
        let d = &diags(&src)[0];
        match fixed {
            Some(fixed) => {
                assert!(d.fixes().iter().all(Fix::certain), "{src}");
                let fixed = format!("{fixed}\nfn main() {{}}");
                assert_eq!(applied(&src, d), fixed);
                clean(&fixed);
            }
            None => assert_eq!(fixes(&src, d), [], "{src}"),
        }
    }
}

/// `sum` adds an `[f64]` too, into an `f64`, and the `sum(xs)` idiom points to it.
#[test]
fn float_sum() {
    contains(
        "fn f(xs: [f64]) {\n    let t: f64 = xs.sum()\n    print(t)\n}\nfn main() {}",
        &["(let t#1 (sum xs#0))"],
    );
    let src = "fn f(xs: [f64]) {\n    print(sum(xs))\n}\nfn main() {}";
    let d = diagnostic(src, Code::MethodAsFunction, (2, 11, "sum"), &[]);
    assert_eq!(
        applied(src, &d),
        "fn f(xs: [f64]) {\n    print(xs.sum())\n}\nfn main() {}"
    );
}

/// Interpolation writes an `f64`; an `f64?` keeps the "cannot interpolate" error.
#[test]
fn float_interpolation() {
    contains(
        "fn f(x: f64) {\n    print(\"{x}!\")\n}\nfn main() {}",
        &["(interp x#0 \"!\")"],
    );
    codes(
        "fn f(x: f64?) {\n    print(\"{x}\")\n}\nfn main() {}",
        &[(Code::CannotInterpolate, (2, 13))],
    );
}

/// One method of each result shape. The declared type proves the result: the same call is a
/// mismatch against the wrong type, which every row names.
#[test]
fn method_types() {
    let rows = [
        ("x.sqrt()", "f64", "bool"),
        ("x.pow(2.0)", "f64", "bool"),
        ("x.hypot(1.0)", "f64", "bool"),
        ("x.is_nan()", "bool", "f64"),
        ("x.is_finite()", "bool", "f64"),
        ("x.is_infinite()", "bool", "f64"),
        ("x.min(1.0)", "f64", "bool"),
        ("x.clamp(0.0, 1.0)", "f64", "bool"),
        ("n.min(1)", "i64", "bool"),
        ("n.max(1)", "i64", "f64"),
        ("n.clamp(0, 1)", "i64", "f64"),
    ];
    for (expr, ty, wrong) in rows {
        let program = |declared: &str| {
            format!(
                "fn f(n: i64, x: f64) {{\n    let y: {declared} = {expr}\n    print(y)\n}}\nfn main() {{}}"
            )
        };
        clean(&program(ty));
        let src = program(wrong);
        let found: Vec<_> = diags(&src).iter().map(Diagnostic::code).collect();
        assert_eq!(found, [Code::Mismatch], "{src}");
    }
    // Through `?.` the result is optional.
    clean("fn f(x: f64?) {\n    let y: f64? = x?.sqrt()\n    print(y ?? 0.0)\n}\nfn main() {}");
    codes(
        "fn f(x: f64?) {\n    let y: f64 = x?.sqrt()\n    print(y)\n}\nfn main() {}",
        &[(Code::Mismatch, (2, 18))],
    );
}

/// The receiver's type is the expected type of each argument, so an integer literal is an
/// `f64`; the receiver comes first among the operands.
#[test]
fn method_literals() {
    contains(
        "fn f(x: f64) {\n    print(x.pow(2))\n}\nfn main() {}",
        &["(math pow x#0 2.0)"],
    );
    contains(
        "fn f(x: f64) {\n    print(x.clamp(0, 1))\n}\nfn main() {}",
        &["(math clamp x#0 0.0 1.0)"],
    );
    contains(
        "fn f(n: i64) {\n    print(n.clamp(0, 1))\n}\nfn main() {}",
        &["(math clamp n#0 0 1)"],
    );
}

/// `Math` prints as `(math <method> <receiver> <arguments>)`, and a literal receiver is the
/// first operand.
#[test]
fn math_prints() {
    contains(
        "fn f(x: f64) {\n    print(x.sqrt())\n    print((-1.0).is_nan())\n    print(x.is_infinite())\n}\nfn main() {}",
        &[
            "(math sqrt x#0)",
            "(math is_nan -1.0)",
            "(math is_infinite x#0)",
        ],
    );
}

#[test]
fn method_errors() {
    let rows = [
        // The count is the method's, as for any method.
        ("x.sqrt(1)", Code::ArgumentCount, (2, 13)),
        ("x.pow()", Code::ArgumentCount, (2, 13)),
        ("x.clamp(1.0)", Code::ArgumentCount, (2, 13)),
        ("x.sqrtt()", Code::NoMember, (2, 15)),
        // `sqrt` is not a method of an `i64`.
        ("n.sqrt()", Code::NoMember, (2, 15)),
        ("b.min(b)", Code::NoMember, (2, 15)),
    ];
    for (expr, code, at) in rows {
        let src = scalars(expr);
        codes(&src, &[(code, at)]);
    }
    // The nearest method is the guess.
    let src = scalars("x.sqrtt()");
    let d = diagnostic(&src, Code::NoMember, (2, 15, "sqrtt"), &[]);
    assert_eq!(
        fixes(&src, &d),
        [(false, vec![(2, 15, "sqrtt", "sqrt".to_owned())])]
    );
    // An argument of the wrong type is a mismatch at the argument, whatever the method.
    codes(&scalars("x.pow(n)"), &[(Code::Mismatch, (2, 19))]);
    // The call that failed is `Error`, so the `bool` it cannot be is not reported too: a bad
    // argument, or a label, is the whole of the report.
    for (arg, code) in [("n", Code::Mismatch), ("v: 1.0", Code::LabeledArgument)] {
        let src = format!(
            "fn f(n: i64, x: f64) {{\n    let y: bool = x.pow({arg})\n    print(y)\n}}\nfn main() {{}}"
        );
        codes(&src, &[(code, (2, 25))]);
    }
}
