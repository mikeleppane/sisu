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
    // A `u8` or an `i64` beside the `char` is not reported as well.
    codes(
        &with_main("fn f(c: char, b: u8, n: i64) {\n    print(b + c)\n    print(n * c)\n}"),
        &[
            (Code::ArithmeticOnChar, (2, 15)),
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
        ("print(i64(n + 1))", "print((n + 1))"),
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
        let edits: Vec<_> = d.fixes()[0].edits().iter().collect();
        assert_eq!(apply(&src, &edits), scalars(fixed), "{body}");
    }
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

/// The number is the mistake, whichever side it is on.
#[test]
fn char_beside_a_number_in_equality() {
    for (body, at) in [
        ("print(1 == c)", (2, 11)),
        ("print(c != n)", (2, 16)),
        ("print(c == b)", (2, 16)),
        ("print(n == c)", (2, 11)),
        ("print(b != c)", (2, 11)),
    ] {
        codes(&scalars(body), &[(Code::Mismatch, at)]);
    }
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
