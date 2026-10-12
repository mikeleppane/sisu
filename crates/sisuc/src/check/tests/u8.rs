//! `u8`: literals typed by context, conversions, and no mixing with `i64`.

use super::*;

/// The help of each diagnostic of `src`, in order.
fn helps(src: &str) -> Vec<Option<String>> {
    diags(src).into_iter().map(|d| d.help).collect()
}

#[test]
fn literal_from_each_source() {
    contains("fn main() { let x: u8 = 200 }", &["200u8"]);
    contains(
        "fn f(b: u8) -> u8 { b }\nfn main() { print(f(7)) }",
        &["(call f 7u8)"],
    );
    contains("fn g() -> u8 { 3 }\nfn main() {}", &["3u8"]);
    contains(
        "class P {\n    b: u8\n}\nfn main() { let p = P(b: 1) }",
        &["(new P 1u8)"],
    );
    contains(
        "fn main() {\n    let mut y: u8 = 0\n    y = 5\n}",
        &["(= y#0 5u8)"],
    );
}

#[test]
fn expected_flows_into_arithmetic() {
    contains("fn main() { let x: u8 = 1 + 2 }", &["(+ 1u8 2u8)"]);
}

#[test]
fn literal_typed_by_sibling() {
    contains(
        "fn f(b: u8, m: u8?, c: bool) {\n    print(b + 1)\n    print(1 + b)\n    print(b != 0)\n    print(0 < b < 10)\n    print(m == 5)\n    print(1 + 2 + b)\n    print(if c { 1 } else { b })\n}\nfn main() {}",
        &[
            "(+ b#0 1u8)",
            "(+ 1u8 b#0)",
            "(!= b#0 0u8)",
            "(< 0u8 b#0 < 10u8)",
            "(== m#1 (wrap 5u8))",
            "(+ (+ 1u8 2u8) b#0)",
            "(block 1u8)",
        ],
    );
}

#[test]
fn optional_literal() {
    contains("fn main() { let m: u8? = 1 }", &["(wrap 1u8)"]);
}

#[test]
fn branch_order() {
    contains(
        "fn f(c: bool, m: u8?) {\n    let a = if c { 1 } else { None }\n    let b = if c { None } else { 1 }\n    print(5 == m)\n}\nfn main() {}",
        &[
            "(let a#2 (if c#0 (block (wrap 1)) (block None)))",
            "(let b#3 (if c#0 (block None) (block (wrap 1))))",
            "(== (wrap 5u8) m#1)",
        ],
    );
}

#[test]
fn compound_assign_types_literal() {
    contains(
        "fn main() {\n    let mut c: u8 = 0\n    c += 250\n}",
        &["(= c#0 (+ c#0 250u8))"],
    );
    contains(
        "class P {\n    mut b: u8\n}\nfn main() {\n    let p = P(b: 0)\n    p.b += 250\n}",
        &["(= (. #1 b) (+ (. #1 b) 250u8))"],
    );
}

#[test]
fn conversions() {
    contains(
        "fn f(b: u8, n: i64) -> i64 { i64(b) + i64(u8(n)) }\nfn main() {}",
        &["(i64 b#0)", "(i64 (u8 n#1))"],
    );
}

#[test]
fn literal_does_not_fit() {
    let message = "`300` does not fit in `u8`";
    errors("fn main() { let x: u8 = 300 }", &[(message, (1, 25))]);
    errors(
        "fn f(b: u8) { print(b + 300) }\nfn main() {}",
        &[(message, (1, 25))],
    );
}

#[test]
fn u8_cannot_be_negative() {
    let message = "`u8` cannot be negative";
    errors("fn main() { let x: u8 = -1 }", &[(message, (1, 25))]);
    // One diagnostic: the literal's size is not reported as well.
    errors("fn main() { let x: u8 = -300 }", &[(message, (1, 25))]);
    errors(
        "fn f(b: u8) { print(-b) }\nfn main() {}",
        &[(message, (1, 21))],
    );
}

#[test]
fn no_mixing() {
    let src = "fn f(b: u8, x: i64) {\n    print(b + x)\n    print(x + b)\n    print(b == x)\n    print(b < x)\n}\nfn main() {}";
    let (to_u8, to_i64) = ("expected `u8`, found `i64`", "expected `i64`, found `u8`");
    errors(
        src,
        &[
            (to_u8, (2, 15)),
            (to_i64, (3, 15)),
            (to_u8, (4, 16)),
            (to_u8, (5, 15)),
        ],
    );
    let (u8_x, i64_b) = ("convert it: `u8(x)`", "convert it: `i64(b)`");
    assert_eq!(
        helps(src),
        [u8_x, i64_b, u8_x, u8_x].map(|h| Some(h.to_owned()))
    );
}

#[test]
fn help_without_path() {
    expect_error(
        "fn n() -> i64 { 1 }\nfn f(b: u8) { print(b + n()) }\nfn main() {}",
        "expected `u8`, found `i64`",
        (2, 25, "n()"),
        None,
        &[],
        Some("convert it: `u8(...)`"),
    );
}

#[test]
fn already_that_type() {
    errors(
        "fn f(x: i64, b: u8) {\n    print(i64(x))\n    print(u8(b))\n    print(i64(5))\n}\nfn main() {}",
        &[
            ("`x` is already `i64`", (2, 15)),
            ("`b` is already `u8`", (3, 14)),
            ("this value is already `i64`", (4, 15)),
        ],
    );
}

#[test]
fn conversion_argument_type() {
    errors(
        "fn main() { print(u8(true)) }",
        &[("expected `i64`, found `bool`", (1, 22))],
    );
}

#[test]
fn conversion_arity() {
    errors(
        "fn main() { print(u8(1, 2)) }",
        &[("`u8` takes 1 argument, found 2", (1, 19))],
    );
}

#[test]
fn built_in_names() {
    for (src, name, col) in [
        ("fn u8() {}", "u8", 4),
        ("fn i64() {}", "i64", 4),
        ("fn bool() {}", "bool", 4),
        ("class u8 {}", "u8", 7),
    ] {
        let message = format!("`{name}` is a built-in type");
        errors(&format!("{src}\nfn main() {{}}"), &[(&message, (1, col))]);
    }
}

#[test]
fn print_takes_u8() {
    clean("fn f(b: u8) { print(b) }\nfn main() {}");
}
