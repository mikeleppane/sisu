//! `string`: literals, `+`, comparison, `len()`, and the operators and tests it does not take.

use super::*;

/// `src` with an empty `main` after it.
fn with_main(src: &str) -> String {
    format!("{src}\nfn main() {{}}")
}

/// A function `f` over `params`, whose body is the one statement `body`, on line 2.
fn body(params: &str, body: &str) -> String {
    with_main(&format!("fn f({params}) {{\n    {body}\n}}"))
}

#[test]
fn string_values() {
    contains(
        "fn f(s: string, m: string?) {\n    print(s.len())\n    print(s == \"x\")\n    print(s < \"b\" <= s)\n    print(m?.len() ?? 0)\n    let xs: [string] = [s, \"y\"]\n    let ys = [\"a\"; 3]\n}\nfn main() {\n    let s = \"ab\" + \"c\"\n}",
        &["(concat \"ab\" \"c\")", "(len s#0)"],
    );
}

/// A chain over `string`s binds each operand once, and stops at the first pair that fails.
#[test]
fn chain_binds_each_operand() {
    contains(
        "fn f(a: string, b: string, c: string) {\n    print(a < b < c)\n}\nfn main() {}",
        &[
            "(print (block (let #3 a#0) (let #4 b#1) (&& (< #3 #4) (block (let #5 c#2) (< #4 #5)))))",
        ],
    );
}

/// An operand of `+` that exits after a `string` one binds the `string` first.
#[test]
fn concat_spills() {
    contains(
        "fn f(c: bool, a: string, b: string) -> string {\n    a + (if c { return \"\" } else { b })\n}\nfn main() {}",
        &["(block (let #3 a#1) (let #4 (if c#0", "(concat #3 #4)"],
    );
}

#[test]
fn concat_mismatch() {
    let params = "s: string, c: char, n: i64, xs: [i64]";
    // The operand reported, and the statement the guess makes of line 2, if there is one.
    let rows = [
        ("print(s + c)", (2, 15, "c"), Some("print(\"{s}{c}\")")),
        ("print(1 + s)", (2, 11, "1"), Some("print(\"{1}{s}\")")),
        // A `char` before a `string` is joined, not arithmetic.
        ("print(c + s)", (2, 11, "c"), Some("print(\"{c}{s}\")")),
        (
            "print(\"count: \" + n)",
            (2, 23, "n"),
            Some("print(\"count: {n}\")"),
        ),
        (
            "print(n + \" items\")",
            (2, 11, "n"),
            Some("print(\"{n} items\")"),
        ),
        ("print(s + xs)", (2, 15, "xs"), None),
        // A compound assignment is a statement, which a rewrite would break.
        ("let mut t = s\n    t += n", (3, 10, "n"), None),
        ("print(s - s)", (2, 11, "s"), None),
    ];
    for (statement, pos, fixed) in rows {
        let src = body(params, statement);
        let d = diagnostic(&src, Code::Mismatch, pos, &[]);
        let found = fixes(&src, &d);
        match fixed {
            Some(fixed) => {
                assert_eq!(found.len(), 1, "{src}");
                assert!(!found[0].0, "{src}: the fix is a guess");
                assert_eq!(applied(&src, &d), body(params, fixed));
            }
            None => assert_eq!(found, [], "{src}"),
        }
    }
}

/// A join with an unknown operand is still a `string`; one with a mismatched operand is
/// `Error`, so its uses report nothing more.
#[test]
fn failed_concat_type() {
    let params = "s: string, xs: [i64]";
    codes(
        &with_main(&format!(
            "fn f({params}) {{\n    let t = s + nope\n    print(t - 1)\n}}"
        )),
        &[(Code::BindingNotFound, (2, 17)), (Code::Mismatch, (3, 11))],
    );
    codes(
        &with_main(&format!(
            "fn f({params}) {{\n    let t = s + xs\n    print(t - 1)\n}}"
        )),
        &[(Code::Mismatch, (2, 17))],
    );
}

/// An ordering is held to its first operand's type; `==` reports the pair.
#[test]
fn string_comparison_mismatch() {
    let params = "s: string, n: i64";
    for (statement, code, pos) in [
        ("print(s < n)", Code::Mismatch, (2, 15, "n")),
        ("print(n < s)", Code::Mismatch, (2, 15, "s")),
        ("print(s == n)", Code::CannotCompare, (2, 11, "s == n")),
    ] {
        let src = body(params, statement);
        let d = diagnostic(&src, code, pos, &[]);
        assert_eq!(fixes(&src, &d), [], "{src}");
    }
}

#[test]
fn compound_on_string_field() {
    let class = "class P {\n    mut name: string\n}\nfn f(p: P) {\n";
    contains(
        &with_main(&format!("{class}    p.name += \"x\"\n}}")),
        &["(= (. #1 name) (concat (. #1 name) \"x\"))"],
    );
    codes(
        &with_main(&format!("{class}    p.name -= \"x\"\n}}")),
        &[(Code::Mismatch, (5, 5))],
    );
}

#[test]
fn string_is() {
    let params = "s: string, m: string?, n: string?";
    for statement in ["print(s is s)", "print(m is n)", "print(s is m)"] {
        codes(&body(params, statement), &[(Code::IsNeedsObjects, (2, 11))]);
    }
}

/// `len()` is the one method until `chars()` and `bytes()` land; a near miss offers it.
#[test]
fn string_methods() {
    for (method, fix) in [("push", None), ("chars", None), ("lem", Some("len"))] {
        let src = format!("fn main() {{\n    \"a\".{method}()\n}}");
        let d = diagnostic(&src, Code::NoMember, (2, 9, method), &[]);
        let expected: Vec<FixAt> = fix
            .map(|fix| (false, vec![(2, 9, method, fix.to_owned())]))
            .into_iter()
            .collect();
        assert_eq!(fixes(&src, &d), expected);
    }
}

#[test]
fn string_built_in_names() {
    codes(
        "fn string() {}\nfn main() {}",
        &[(Code::BuiltInTypeName, (1, 4))],
    );
    codes(
        "class string {}\nfn main() {}",
        &[(Code::BuiltInTypeName, (1, 7))],
    );
    // `string(cs)` lands with `chars()`; until then it names no function.
    codes(
        "fn main() {\n    print(string(1))\n}",
        &[(Code::FunctionNotFound, (2, 11))],
    );
}
