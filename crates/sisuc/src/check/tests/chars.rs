//! `char`: literals, comparison, conversions, and no arithmetic.

use super::*;

/// `src` with an empty `main` after it.
fn with_main(src: &str) -> String {
    format!("{src}\nfn main() {{}}")
}

/// A function `f` over every scalar type, whose body is the one statement `body`, on line 2.
fn scalars(body: &str) -> String {
    with_main(&format!(
        "fn f(c: char, b: u8, x: bool, n: i64) {{\n    {body}\n}}"
    ))
}

/// The one diagnostic of `src`, its fixes, and `src` with the first applied.
fn only_fix<'a>(
    src: &'a str,
    code: Code,
    pos: (usize, usize, &str),
) -> (Diagnostic, Vec<FixAt<'a>>) {
    let d = diagnostic(src, code, pos, &[]);
    let found = fixes(src, &d);
    (d, found)
}

#[test]
fn char_values() {
    contains(
        "fn f(c: char, m: char?) {\n    print(c)\n    print(i64(c))\n    print(char(228))\n    print(c < 'z' <= c)\n    print(m == 'a')\n    let cs: [char] = ['a']\n}\nfn main() {\n    let c = 'ä'\n}",
        &["'ä'", "(i64 c#0)", "(char 228)"],
    );
}

#[test]
fn char_comparison_takes_chars() {
    contains(
        "fn f(c: char, d: char) {\n    print(c == d)\n    print(c != d)\n    print(c < d)\n    print(c >= d)\n}\nfn main() {}",
        &[
            "(== c#0 d#1)",
            "(!= c#0 d#1)",
            "(< c#0 d#1)",
            "(>= c#0 d#1)",
        ],
    );
}

#[test]
fn char_arithmetic() {
    let edit = |line, col, replacement: &str| (line, col, "", replacement.to_owned());
    let rows = [
        (
            "fn f(c: char) {\n    print(c + 1)\n}",
            (2, 11, "c"),
            Some(vec![
                edit(2, 11, "char(i64("),
                edit(2, 12, ")"),
                edit(2, 16, ")"),
            ]),
        ),
        (
            "fn f(c: char) {\n    print(1 + c)\n}",
            (2, 15, "c"),
            Some(vec![
                edit(2, 11, "char("),
                edit(2, 15, "i64("),
                edit(2, 16, "))"),
            ]),
        ),
        (
            "fn f(c: char) {\n    print(c * c)\n}",
            (2, 11, "c"),
            Some(vec![
                edit(2, 11, "char(i64("),
                edit(2, 12, ")"),
                edit(2, 15, "i64("),
                edit(2, 16, "))"),
            ]),
        ),
        // A compound assignment is a statement, which a rewrite would break.
        (
            "fn f(c: char) {\n    let mut d = c\n    d += 1\n}",
            (3, 5, "d"),
            None,
        ),
        (
            "class P {\n    mut c: char\n}\nfn f(p: P) {\n    p.c -= 1\n}",
            (5, 5, "p.c"),
            None,
        ),
        // A `char` on the right of a number's compound assignment is the one reported, in
        // each place the number can be: a local, a field and an element.
        (
            "fn f(c: char) {\n    let mut n = 1\n    n += c\n}",
            (3, 10, "c"),
            None,
        ),
        (
            "class P {\n    mut n: i64\n}\nfn f(p: P, c: char) {\n    p.n += c\n}",
            (5, 12, "c"),
            None,
        ),
        (
            "fn f(c: char) {\n    let mut a = [1]\n    a[0] += c\n}",
            (3, 13, "c"),
            None,
        ),
        // The `char` on the left is reported first, whatever stands on the right.
        (
            "fn f(c: char) {\n    let mut d = c\n    d += c\n}",
            (3, 5, "d"),
            None,
        ),
        (
            "fn f(c: char) {\n    let mut a = [c]\n    a[0] *= 2\n}",
            (3, 5, "a[0]"),
            None,
        ),
        // A compound assignment nested in the value leaves the outer one a statement still.
        (
            "fn f(c: char) {\n    let mut n = 1\n    let mut m = 1\n    n += {\n        m += 1\n        c\n    }\n}",
            (4, 10, "{\n        m += 1\n        c\n    }"),
            None,
        ),
        (
            "fn f(c: char) {\n    let mut n = 1\n    let mut m = 1\n    n += if true {\n        m += 1\n        c\n    } else {\n        c\n    }\n}",
            (
                4,
                10,
                "if true {\n        m += 1\n        c\n    } else {\n        c\n    }",
            ),
            None,
        ),
    ];
    for (src, pos, edits) in rows {
        let src = with_main(src);
        let (d, found) = only_fix(&src, Code::ArithmeticOnChar, pos);
        match edits {
            Some(edits) => {
                assert_eq!(found, [(false, edits)], "{src}");
                // Whatever else the guess leaves, it is no longer arithmetic on a `char`.
                let result = applied(&src, &d);
                assert!(
                    !diags(&result)
                        .iter()
                        .any(|d| d.code() == Code::ArithmeticOnChar)
                );
            }
            None => assert_eq!(found, [], "{src}"),
        }
    }
}

/// A compound assignment inside an ordinary `+` is reported without a fix, and the `+` keeps
/// its guess.
#[test]
fn char_compound_inside_arithmetic() {
    let src = with_main(
        "fn f(c: char) {\n    let mut n = 1\n    print(c + {\n        n += c\n        1\n    })\n}",
    );
    let ds = diags(&src);
    let found: Vec<_> = ds
        .iter()
        .map(|d| {
            (
                d.code(),
                at(&src, d.span()),
                fixes(&src, d).iter().map(|f| f.0).collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            (Code::ArithmeticOnChar, (3, 11), vec![false]),
            (Code::ArithmeticOnChar, (4, 14), vec![]),
        ]
    );
}

/// The rewrite is what the types need, in each place the operator stands.
#[test]
fn char_arithmetic_fix_types() {
    for (src, at, fixed) in [
        (
            "fn f(c: char) {\n    let d = c + 1\n}",
            (2, 13, "c"),
            "fn f(c: char) {\n    let d = char(i64(c) + 1)\n}",
        ),
        (
            "fn f(c: char, n: i64) {\n    let d: char = n - c\n}",
            (2, 23, "c"),
            "fn f(c: char, n: i64) {\n    let d: char = char(n - i64(c))\n}",
        ),
    ] {
        let (src, fixed) = (with_main(src), with_main(fixed));
        let d = diagnostic(&src, Code::ArithmeticOnChar, at, &[]);
        assert_eq!(applied(&src, &d), fixed);
        clean(&fixed);
    }
}

#[test]
fn char_arithmetic_reports_once() {
    // A longer expression reports its first `char` operand, and nothing above it.
    codes(
        &with_main("fn f(c: char) {\n    print(c + 1 + 2)\n}"),
        &[(Code::ArithmeticOnChar, (2, 11))],
    );
    // A `u8` or an `i64` beside the `char` is not reported as well, on either side.
    codes(
        &with_main(
            "fn f(c: char, b: u8, n: i64) {\n    print(b + c)\n    print(n * c)\n    print(c + b)\n    print(c - n)\n    print(b + c + b)\n}",
        ),
        &[
            (Code::ArithmeticOnChar, (2, 15)),
            (Code::ArithmeticOnChar, (3, 15)),
            (Code::ArithmeticOnChar, (4, 11)),
            (Code::ArithmeticOnChar, (5, 11)),
            (Code::ArithmeticOnChar, (6, 15)),
        ],
    );
    // Nor is a `bool` or a `None`, on either side, nor a literal that a `u8` context types.
    codes(
        &with_main(
            "fn f(c: char) {\n    print(true + c)\n    print(c + true)\n    print(None + c)\n    print(c + None)\n    let y: u8 = None + c\n    let z: u8 = 300 + c\n}",
        ),
        &[
            (Code::ArithmeticOnChar, (2, 18)),
            (Code::ArithmeticOnChar, (3, 11)),
            (Code::ArithmeticOnChar, (4, 18)),
            (Code::ArithmeticOnChar, (5, 11)),
            (Code::ArithmeticOnChar, (6, 24)),
            (Code::ArithmeticOnChar, (7, 23)),
        ],
    );
    // An error of the other operand stays: an unknown name is its own mistake.
    codes(
        &with_main("fn f(c: char) {\n    print(c + z)\n    print(z + c)\n}"),
        &[
            (Code::ArithmeticOnChar, (2, 11)),
            (Code::BindingNotFound, (2, 15)),
            (Code::BindingNotFound, (3, 11)),
            (Code::ArithmeticOnChar, (3, 15)),
        ],
    );
}

#[test]
fn char_conversions() {
    let edit = |col, replaced, replacement: &str| (2, col, replaced, replacement.to_owned());
    // Where the conversion is reported, and its fix.
    let rows: [(_, _, _, Vec<FixAt>); 6] = [
        (
            "print(char(c))",
            (Code::AlreadyConverted, (2, 16, "c")),
            "print(c)",
            vec![(true, vec![edit(11, "char(", ""), edit(17, ")", "")])],
        ),
        (
            "print(char(b))",
            (Code::Mismatch, (2, 16, "b")),
            "print(char(i64(b)))",
            vec![(false, vec![edit(16, "", "i64("), edit(17, "", ")")])],
        ),
        (
            "print(u8(c))",
            (Code::Mismatch, (2, 14, "c")),
            "print(u8(i64(c)))",
            vec![(false, vec![edit(14, "", "i64("), edit(15, "", ")")])],
        ),
        ("print(i64(x))", (Code::Mismatch, (2, 15, "x")), "", vec![]),
        (
            "print(i64(n + 1) * 2)",
            (Code::AlreadyConverted, (2, 15, "n + 1")),
            "print((n + 1) * 2)",
            vec![(true, vec![edit(11, "i64(", "(")])],
        ),
        (
            "let y = i64({ n })",
            (Code::AlreadyConverted, (2, 17, "{ n }")),
            "let y = ({ n })",
            vec![(true, vec![edit(13, "i64(", "(")])],
        ),
    ];
    for (body, (code, pos), fixed, expected) in rows {
        let src = scalars(body);
        let d = diagnostic(&src, code, pos, &[]);
        assert_eq!(fixes(&src, &d), expected, "{src}");
        if code == Code::AlreadyConverted {
            fix_removes_its_code(&src, &d);
            assert_eq!(applied(&src, &d), scalars(fixed));
        } else if !expected.is_empty() {
            assert_eq!(applied(&src, &d), scalars(fixed));
            clean(&scalars(fixed));
        }
    }
}

/// Removing a conversion drops its parentheses around one term, and keeps them around what is
/// not one: a parenthesized operand is one term, and an operand made of terms is not.
#[test]
fn already_converted_groups_what_is_not_one_term() {
    for (body, fixed) in [
        ("print(i64(n))", "print(n)"),
        ("print(i64((n)))", "print((n))"),
        ("print(i64((n + 1)))", "print((n + 1))"),
        ("print(i64((n) + (1)) * 2)", "print(((n) + (1)) * 2)"),
        ("print(i64(-n))", "print((-n))"),
        ("print(i64(-(n)))", "print((-(n)))"),
        (
            "print(i64(if x { n } else { 1 }))",
            "print((if x { n } else { 1 }))",
        ),
    ] {
        let src = scalars(body);
        let d = &diags(&src)[0];
        assert_eq!(d.code(), Code::AlreadyConverted, "{src}");
        fix_removes_its_code(&src, d);
        let edits: Vec<_> = d.fixes()[0].edits().iter().collect();
        assert_eq!(apply(&src, &edits), scalars(fixed), "{body}");
    }
}

/// The fix deletes what stands between the conversion and its operand, so a comment there makes
/// it a guess: the person may want it kept.
#[test]
fn already_converted_with_a_comment_is_a_guess() {
    for (body, fixed) in [
        ("print(i64(\n        // c\n        n\n    ))", "print(n)"),
        ("print(i64(n // t\n    ))", "print(n)"),
        (
            "print(i64(\n        // c\n        n + 1\n    ))",
            "print((n + 1\n    ))",
        ),
    ] {
        let src = scalars(body);
        let d = &diags(&src)[0];
        assert_eq!(d.code(), Code::AlreadyConverted, "{src}");
        let found = fixes(&src, d);
        assert_eq!(found.len(), 1, "{src}");
        assert!(!found[0].0, "{src}");
        assert_eq!(applied(&src, d), scalars(fixed), "{body}");
    }
}

/// A comment the removal keeps, or that stands outside the conversion, does not demote the fix.
#[test]
fn already_converted_keeps_comments_it_does_not_delete() {
    for body in [
        "print(i64(n)) // after",
        "// before\n    print(i64(n))",
        // The grouped tail stays, with its comment.
        "print(i64(n + 1 // t\n    ) * 2)",
    ] {
        let src = scalars(body);
        let d = &diags(&src)[0];
        assert_eq!(d.code(), Code::AlreadyConverted, "{src}");
        fix_removes_its_code(&src, d);
    }
}

/// A label is its own mistake, which the fix would delete with the conversion: none is offered.
#[test]
fn already_converted_with_a_label_has_no_fix() {
    let src = scalars("print(i64(x: n))");
    let ds = diags(&src);
    let found: Vec<_> = ds.iter().map(|d| (d.code(), at(&src, d.span()))).collect();
    assert_eq!(
        found,
        [
            (Code::LabeledArgument, (2, 15)),
            (Code::AlreadyConverted, (2, 18)),
        ]
    );
    assert_eq!(fixes(&src, &ds[1]), []);
}

#[test]
fn char_comparison_mismatch() {
    let edit = |col, replacement: &str| (2, col, "", replacement.to_owned());
    for (body, col) in [("print(c == 1)", 16), ("print(c < n)", 15)] {
        let src = scalars(body);
        let d = diagnostic(&src, Code::Mismatch, (2, col, &body[col - 5..col - 4]), &[]);
        assert_eq!(
            fixes(&src, &d),
            [(false, vec![edit(col, "char("), edit(col + 1, ")")])]
        );
    }
    // Another type beside a `char` is a different comparison, not a missing conversion.
    codes(
        &scalars("print(c == true)"),
        &[(Code::CannotCompare, (2, 11))],
    );
}

/// The number is the mistake, whichever side it is on and whichever comparison it is in.
#[test]
fn char_beside_a_number_in_equality() {
    for (body, at) in [
        ("print(1 == c)", (2, 11)),
        ("print(c != n)", (2, 16)),
        ("print(c == b)", (2, 16)),
        ("print(n == c)", (2, 11)),
        ("print(b != c)", (2, 11)),
        ("print(n < c)", (2, 11)),
        ("print(b < c)", (2, 11)),
        ("print(c < n)", (2, 15)),
        ("print(c >= b)", (2, 16)),
        ("print(1 < c)", (2, 11)),
        ("print(n < n <= c)", (2, 11)),
    ] {
        codes(&scalars(body), &[(Code::Mismatch, at)]);
    }
    // Its fix converts that number to a `char`.
    for (body, col, open, close) in [
        ("print(n < c)", 11, "char(", ")"),
        ("print(b < c)", 11, "char(i64(", "))"),
    ] {
        let src = scalars(body);
        let d = diagnostic(&src, Code::Mismatch, (2, col, &body[col - 5..col - 4]), &[]);
        let edit = |col, replacement: &str| (2, col, "", replacement.to_owned());
        assert_eq!(
            fixes(&src, &d),
            [(false, vec![edit(col, open), edit(col + 1, close)])]
        );
    }
}

/// `-c` is a number's sign, which a conversion fixes.
#[test]
fn char_negated() {
    let src = scalars("print(-c)");
    let d = diagnostic(&src, Code::Mismatch, (2, 12, "c"), &[]);
    assert_eq!(
        fixes(&src, &d),
        [(
            false,
            vec![(2, 12, "", "i64(".to_owned()), (2, 13, "", ")".to_owned())]
        )]
    );
    assert_eq!(applied(&src, &d), scalars("print(-i64(c))"));
    clean(&applied(&src, &d));
}

/// `!c` wants a `bool`, which no conversion makes.
#[test]
fn char_not() {
    let src = scalars("print(!c)");
    let d = diagnostic(&src, Code::Mismatch, (2, 12, "c"), &[]);
    assert_eq!(fixes(&src, &d), []);
}

/// An optional `char` is the optional's mistake, and its arithmetic is not reported.
#[test]
fn optional_char_arithmetic() {
    codes(
        &with_main("fn f(m: char?) {\n    print(m + 1)\n}"),
        &[(Code::Mismatch, (2, 11))],
    );
}

/// A copy of a `char` is a `char`: it is not assignable, and not arithmetic.
#[test]
fn char_copy_compound_assignment() {
    codes(
        &with_main("fn f(c: char) {\n    let d = c\n    d += 1\n}"),
        &[
            (Code::ArithmeticOnChar, (3, 5)),
            (Code::AssignToImmutable, (3, 5)),
        ],
    );
}

#[test]
fn char_built_in_names() {
    for (src, col) in [("fn char() {}", 4), ("class char {}", 7)] {
        codes(&with_main(src), &[(Code::BuiltInTypeName, (1, col))]);
    }
}

#[test]
fn char_mismatch() {
    let edit = |col, replacement: &str| (2, col, "", replacement.to_owned());
    let rows = [
        ("let x: char = b", 19, "char(i64(", "))"),
        ("let y: u8 = c", 17, "u8(i64(", "))"),
        ("let z: i64 = c", 18, "i64(", ")"),
        ("let w: char = n", 19, "char(", ")"),
        ("let v: char = 1", 19, "char(", ")"),
    ];
    for (body, col, open, close) in rows {
        let src = scalars(body);
        let (d, found) = only_fix(&src, Code::Mismatch, (2, col, &body[body.len() - 1..]));
        assert_eq!(
            found,
            [(false, vec![edit(col, open), edit(col + 1, close)])],
            "{src}"
        );
        clean(&applied(&src, &d));
    }
}

#[test]
fn char_names_leave_the_did_you_mean_candidates() {
    // `char` is a conversion now: a misspelling of it is offered it, and `char(...)` is found.
    let d = diagnostic(
        &with_main("fn f(n: i64) {\n    print(chr(n))\n}"),
        Code::FunctionNotFound,
        (2, 11, "chr"),
        &[],
    );
    assert_eq!(
        fixes("fn f(n: i64) {\n    print(chr(n))\n}\nfn main() {}", &d),
        [(false, vec![(2, 11, "chr", "char".to_owned())])]
    );
    clean(&with_main("fn f(n: i64) {\n    print(char(n))\n}"));
}

#[test]
fn char_as_a_type_name_suggestion() {
    let src = with_main("fn f(c: chaa) {}");
    let d = diagnostic(&src, Code::UnknownType, (1, 9, "chaa"), &[]);
    assert_eq!(
        fixes(&src, &d),
        [(false, vec![(1, 9, "chaa", "char".to_owned())])]
    );
}
