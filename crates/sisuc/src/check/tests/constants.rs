//! Top-level constants: folded at compile time in source order, and used as their literal.

use super::*;

/// `src`, with an empty `main` after it unless it has one.
fn with_main(src: &str) -> String {
    if src.contains("fn main") {
        src.to_string()
    } else {
        format!("{src}\nfn main() {{}}")
    }
}

#[test]
fn folds() {
    for (consts, value) in [
        ("const L = 100\nconst A = L * L", "10000"),
        ("const A: u8 = 0xF0", "240u8"),
        ("const M: u8 = 0xF0\nconst A = M + 1", "241u8"),
        ("const A = 7 - 9", "-2"),
        ("const A = -7 / 2", "-3"),
        ("const A = -7 % 2", "-1"),
        ("const A: u8 = 200 / 3", "66u8"),
        ("const A: u8 = 200 % 3", "2u8"),
        ("const A = 1 < 3 < 2", "false"),
        ("const A = 3 > 2 > 1", "true"),
        ("const A = 1 <= 1", "true"),
        ("const A = 2 >= 3", "false"),
        ("const A = 1 < 2", "true"),
        ("const A = 1 < 1", "false"),
        ("const A = 2 > 2", "false"),
        ("const A = 2 <= 1", "false"),
        ("const A = 2 >= 2", "true"),
        ("const A = 1 != 2", "true"),
        ("const A = 1 == 1", "true"),
        ("const A = true == false", "false"),
        ("const A = !false && 1 < 2", "true"),
        ("const A = true && false", "false"),
        ("const A = false || true", "true"),
        ("const A = false || false", "false"),
        ("const A = false && 1 / 0 == 1", "false"),
        ("const A = true || 1 / 0 == 1", "true"),
        ("const A = 2 < 1 < 9223372036854775807 + 1", "false"),
        ("const B = 255\nconst A = u8(B)", "255u8"),
        ("const M: u8 = 7\nconst A = i64(M) - 8", "-1"),
        ("const A = -9223372036854775807 - 1", "-9223372036854775808"),
        ("const A: u8 = 255", "255u8"),
        ("const A = 3 - 2 - 1", "0"),
        ("const B = true\nconst A = !B", "false"),
    ] {
        let src = format!("{consts}\nfn main() {{ print(A) }}");
        contains(&src, &[&format!("(print {value})")]);
    }
}

/// A use lowers to the literal at the use, and its type takes part in the checks around it.
#[test]
fn uses_lower_to_literals() {
    contains(
        "const A = 2\nfn main() { print(A * 3) }",
        &["(print (* 2 3))"],
    );
    contains(
        "const A: u8 = 1\nfn main() {\n    let b: u8 = A + 1\n    print(b)\n}",
        &["(let b#0 (+ 1u8 1u8))"],
    );
    clean("const A = true\nfn f() -> bool { A }\nfn main() { print(f()) }");
}

#[test]
fn local_shadows_const() {
    contains(
        "const A = 1\nfn main() { let A = 2\n print(A) }",
        &["(let A#0 2)", "(print A#0)"],
    );
    contains(
        "const A = 1\nfn f(A: bool) { print(A) }\nfn main() { f(true) }",
        &["(print A#0)"],
    );
    codes(
        "const A = 1\nfn main() { let A: Foo = 1\n print(A) }",
        &[(Code::UnknownType, (2, 20))],
    );
    clean("const A = 1\nfn main() { let mut A = 2\n A = 3\n print(A) }");
}

/// A source, with an empty `main` unless it has one, and its exact codes with positions.
type Row<'a> = (&'a str, &'a [(Code, (usize, usize))]);

/// Asserts each row.
fn rows(rows: &[Row]) {
    for (src, expected) in rows {
        codes(&with_main(src), expected);
    }
}

/// The gate reports each part of a value that is not known, and a constant used before it
/// is declared.
#[test]
fn gate_errors() {
    rows(&[
        ("const A = f()", &[(Code::ConstantNotKnown, (1, 11))]),
        ("const A = [1]", &[(Code::ConstantNotKnown, (1, 11))]),
        (
            "const A = true || f()",
            &[(Code::ConstantNotKnown, (1, 19))],
        ),
        (
            "const A = f() + g()",
            &[
                (Code::ConstantNotKnown, (1, 11)),
                (Code::ConstantNotKnown, (1, 17)),
            ],
        ),
        (
            "const A = if true { 1 } else { 2 }",
            &[(Code::ConstantNotKnown, (1, 11))],
        ),
        ("const A = { 1 }", &[(Code::ConstantNotKnown, (1, 11))]),
        ("const A = len([1])", &[(Code::ConstantNotKnown, (1, 11))]),
        ("const A = 1 < A", &[(Code::ConstantOrder, (1, 15))]),
        (
            "const A = i64(B)\nconst B = 1",
            &[(Code::ConstantOrder, (1, 15))],
        ),
        // `B` uses `A`, which failed, so `B` is poisoned and says nothing.
        (
            "const A = B\nconst B = A",
            &[(Code::ConstantOrder, (1, 11))],
        ),
    ]);
}

/// A declared type outside the type list is the only error; an unknown one is `resolve`'s.
#[test]
fn type_list_errors() {
    rows(&[
        ("const A: [i64] = [1]", &[(Code::ConstantType, (1, 10))]),
        ("const A: i64? = 1", &[(Code::ConstantType, (1, 10))]),
        ("const A: Foo = 1", &[(Code::UnknownType, (1, 10))]),
    ]);
}

/// The value is typed as a `let` initializer is.
#[test]
fn typing_errors() {
    rows(&[
        ("const A = true || 5", &[(Code::Mismatch, (1, 19))]),
        ("const A: bool = 1", &[(Code::Mismatch, (1, 17))]),
        ("const A = true + 1", &[(Code::Mismatch, (1, 11))]),
        ("const A: u8 = 300", &[(Code::IntegerDoesNotFit, (1, 15))]),
        ("const A: u8 = -1", &[(Code::NegatedU8, (1, 15))]),
        ("const A = 1 == true", &[(Code::CannotCompare, (1, 11))]),
        ("const A = x", &[(Code::BindingNotFound, (1, 11))]),
    ]);
}

/// What would panic at run time is an error at the failing operation.
#[test]
fn fold_errors() {
    rows(&[
        (
            "const A = 9223372036854775807 + 1",
            &[(Code::ConstantPanics, (1, 11))],
        ),
        (
            "const A = 2 * 4611686018427387904",
            &[(Code::ConstantPanics, (1, 11))],
        ),
        (
            "const A = -9223372036854775807 - 2",
            &[(Code::ConstantPanics, (1, 11))],
        ),
        (
            "const A: u8 = 250 + 10 - 10",
            &[(Code::ConstantPanics, (1, 15))],
        ),
        ("const A: u8 = 1 - 2", &[(Code::ConstantPanics, (1, 15))]),
        ("const A: u8 = 16 * 16", &[(Code::ConstantPanics, (1, 15))]),
        ("const A: u8 = 1 / 0", &[(Code::ConstantPanics, (1, 15))]),
        ("const A = 1 / 0", &[(Code::ConstantPanics, (1, 11))]),
        ("const A = 1 % 0", &[(Code::ConstantPanics, (1, 11))]),
        (
            "const A = 1 < 2 < 1 / 0",
            &[(Code::ConstantPanics, (1, 19))],
        ),
        (
            "const A = 1 < 2 && 1 / 0 == 1",
            &[(Code::ConstantPanics, (1, 20))],
        ),
        ("const A = u8(300)", &[(Code::ConstantPanics, (1, 11))]),
        ("const A = u8(-1)", &[(Code::ConstantPanics, (1, 11))]),
        // Run time evaluates the left operand first, so its panic is the one reported.
        (
            "const A = (1 / 0) + (9223372036854775807 + 1)",
            &[(Code::ConstantPanics, (1, 11))],
        ),
        (
            "const A = 1 / 0 == 9223372036854775807 + 1",
            &[(Code::ConstantPanics, (1, 11))],
        ),
        (
            "const A = 1 / 0 < 9223372036854775807 + 1",
            &[(Code::ConstantPanics, (1, 11))],
        ),
        (
            "const A = 1 < 1 / 0 < 9223372036854775807 + 1",
            &[(Code::ConstantPanics, (1, 15))],
        ),
    ]);
    let min = "const MIN = -9223372036854775807 - 1";
    for value in ["MIN / -1", "MIN % -1", "-MIN", "MIN - 1"] {
        codes(
            &with_main(&format!("{min}\nconst A = {value}")),
            &[(Code::ConstantPanics, (2, 11))],
        );
    }
}

/// A `char` constant folds, compares by code point and converts as run time does.
#[test]
fn const_char() {
    // Each initializer folds on its own, and the prints show them in order: a comparison in
    // `main` would not be folded, and a value that folded wrongly would show in its place.
    let consts = [
        ("C", "'ä'", "'ä'"),
        ("LESS", "'a' < 'b'", "true"),
        ("MORE", "'a' > 'b'", "false"),
        ("SAME", "C == 'ä'", "true"),
        ("OTHER", "C == 'a'", "false"),
        ("DIFFERS", "C != 'a'", "true"),
        ("EQUAL", "C != 'ä'", "false"),
        ("BELOW", "char(55295) < char(57344)", "true"),
        ("ABOVE", "char(57344) < char(55295)", "false"),
        ("TOP", "char(0x10FFFF) >= char(57344)", "true"),
        ("CHAIN", "'a' < 'b' <= 'b'", "true"),
        ("NEXT", "char(i64(C) + 1)", "'å'"),
        ("N", "'x'", "'x'"),
        ("CODE", "i64(C)", "228"),
    ];
    let lines = |line: &dyn Fn(&str, &str) -> String| -> String {
        let lines: Vec<_> = consts
            .iter()
            .map(|(name, value, _)| line(name, value))
            .collect();
        lines.join("")
    };
    let decls = lines(&|name, value| format!("const {name} = {value}\n"));
    let prints = lines(&|name, _| format!("    print({name})\n"));
    let folded: Vec<_> = consts
        .iter()
        .map(|(.., value)| format!("(print {value})"))
        .collect();
    contains(
        &format!("{decls}fn main() {{\n{prints}}}"),
        &[&format!("(block {})", folded.join(" "))],
    );
    // No arithmetic, even in a constant: the guess wraps the constant as it does a local.
    let src = with_main("const C = 'ä'\nconst D = C + 1");
    let d = diagnostic(&src, Code::ArithmeticOnChar, (2, 11, "C"), &[]);
    assert_eq!(
        applied(&src, &d),
        with_main("const C = 'ä'\nconst D = char(i64(C) + 1)")
    );
    // A scalar value converts; the surrogates and everything above `10FFFF` panic as run time does.
    for value in ["0xD800", "0xDFFF", "0x110000", "-1"] {
        let src = with_main(&format!("const F = char({value})"));
        let d = diagnostic(
            &src,
            Code::ConstantPanics,
            (1, 11, &format!("char({value})")),
            &[],
        );
        assert_eq!(fixes(&src, &d), []);
    }
    clean(&with_main(
        "const N: char = 'x'\nconst F = char(0xD7FF)\nconst G = char(0xE000)\nconst H = char(0x10FFFF)",
    ));
}

/// A `string` constant folds `+` and chains as run time does, and each use is its literal.
#[test]
fn const_string() {
    contains(
        "const G = \"good\" + \"bye\"\nconst S: string = \"x\"\nconst L = \"a\" < \"b\" < \"c\"\nconst K = \"b\" < \"a\" < \"c\"\nfn main() {\n    let n = 1\n    print(G)\n    print(G)\n    print(S)\n    print(L)\n    print(K)\n}",
        // A chain's locals stay with the constant: `main`'s first local is still 0.
        &[
            "(let n#0 1) (print \"goodbye\") (print \"goodbye\") (print \"x\") (print true) (print false)",
        ],
    );
    rows(&[
        (
            "const G = \"a\"\nconst N = G.len()",
            &[(Code::ConstantNotKnown, (2, 11))],
        ),
        ("const P = \"a\" + 1", &[(Code::Mismatch, (1, 17))]),
    ]);
    // Interpolation is not known at compile time, so it is not offered.
    let src = with_main("const P = \"a\" + 1");
    let d = diagnostic(&src, Code::Mismatch, (1, 17, "1"), &[]);
    assert_eq!(fixes(&src, &d), []);
}

/// Constants share the namespace of functions and classes.
#[test]
fn name_errors() {
    rows(&[
        // A duplicate goes through every step for its own errors.
        (
            "const A = 1\nconst A = 1 / 0",
            &[
                (Code::DefinedTwice, (2, 7)),
                (Code::ConstantPanics, (2, 11)),
            ],
        ),
        // The first of two constants of one name is the one its uses read.
        (
            "const A = 1\nconst A = true\nfn main() { let b: i64 = A }",
            &[(Code::DefinedTwice, (2, 7))],
        ),
        ("class A {}\nconst A = 1", &[(Code::DefinedTwice, (2, 7))]),
        ("const A = 1\nfn A() {}", &[(Code::DefinedTwice, (2, 4))]),
        ("const print = 1", &[(Code::DefinedTwice, (1, 7))]),
        ("const i64 = 1", &[(Code::BuiltInTypeName, (1, 7))]),
    ]);
}

/// A use is checked as a literal of the constant's type; a failed constant is poisoned.
#[test]
fn use_errors() {
    rows(&[
        (
            "const A = 1\nfn main() { A[0] = 1 }",
            &[
                (Code::AssignToImmutable, (2, 13)),
                (Code::CannotIndex, (2, 13)),
            ],
        ),
        (
            "const A = 1\nfn main() { A() }",
            &[(Code::FunctionNotFound, (2, 13))],
        ),
        (
            "const A = f()\nconst B = A + 1\nfn main() {\n    print(A)\n    print(B)\n}",
            &[(Code::ConstantNotKnown, (1, 11))],
        ),
        (
            "const A: u8 = 1 / 0\nfn main() { let b: bool = A }",
            &[(Code::ConstantPanics, (1, 15)), (Code::Mismatch, (2, 27))],
        ),
    ]);
}

#[test]
fn const_diagnostics() {
    diagnostic(
        "const A = B\nconst B = 1\nfn main() {}",
        Code::ConstantOrder,
        (1, 11, "B"),
        &[(2, 7)],
    );
    diagnostic(
        "const A = A + 1\nfn main() {}",
        Code::ConstantOrder,
        (1, 11, "A"),
        &[(1, 7)],
    );
    diagnostic(
        "const A = 1\nconst A = 2\nfn main() {}",
        Code::DefinedTwice,
        (2, 7, "A"),
        &[(1, 7)],
    );
    for assign in ["A = 2", "A += 2"] {
        diagnostic(
            &format!("const A = 1\nfn main() {{ {assign} }}"),
            Code::AssignToImmutable,
            (2, 13, assign),
            &[(1, 7)],
        );
    }
    diagnostic(
        "const A: u8 = 250 + 10 - 10\nfn main() {}",
        Code::ConstantPanics,
        (1, 15, "250 + 10"),
        &[],
    );
    diagnostic(
        "const A = u8(300)\nfn main() {}",
        Code::ConstantPanics,
        (1, 11, "u8(300)"),
        &[],
    );
}

/// E0032's guess wraps the constant in the conversion, as it does a local.
#[test]
fn mismatch_guess_wraps_the_constant() {
    for (src, wrapped) in [
        ("const I = 1\nconst A: u8 = I\nfn main() {}", "u8(I)"),
        ("const A: u8 = 1\nfn main() { let b: i64 = A }", "i64(A)"),
    ] {
        let ds = diags(src);
        let [d] = ds.as_slice() else {
            panic!("{src}: {ds:?}");
        };
        assert_eq!(d.code(), Code::Mismatch);
        assert!(applied(src, d).contains(wrapped), "{src}");
        assert!(d.fixes().iter().all(|f| !f.certain()), "{src}");
    }
}

/// The `main` rejected as a duplicate still exists, whichever comes first.
#[test]
fn main_beside_a_constant_named_main_is_not_missing() {
    diagnostic(
        "const main = 1\nfn main() {}",
        Code::DefinedTwice,
        (2, 4, "main"),
        &[(1, 7)],
    );
    diagnostic(
        "fn main() {}\nconst main = 1",
        Code::DefinedTwice,
        (2, 7, "main"),
        &[(1, 4)],
    );
}
