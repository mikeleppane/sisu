//! `for` over arrays and ranges: the lowering, what the header accepts, and the loop variable.

use super::*;

#[test]
fn lowers_for_over_array() {
    contains(
        "fn f(xs: [i64]) { for x in xs { print(x) } }\nfn main() {}",
        &[
            "(block (let #1 xs#0) (let #2 (len #1)) (var #3 0) (loop (block (if (< #3 #2) (block (let x#4 (index #1 #3)) (= #3 (+ #3 1)) (block (print x#4))) (block (break))))))",
        ],
    );
}

#[test]
fn lowers_for_over_range() {
    contains(
        "fn f(n: i64) { for i in 0..n { print(i) } }\nfn main() {}",
        &[
            "(block (var #1 0) (let #2 n#0) (loop (block (if (< #1 #2) (block (let i#3 #1) (= #1 (+ #1 1)) (block (print i#3))) (block (break))))))",
        ],
    );
}

#[test]
fn range_bounds_are_i64() {
    errors(
        "fn f(b: u8) { for i in 0..b {} }\nfn main() {}",
        &[("expected `i64`, found `u8`", (1, 27))],
    );
}

#[test]
fn for_needs_array() {
    expect_error(
        "fn main() { for x in 5 {} }",
        "`for` needs an array or a range; this is `i64`",
        (1, 22, "5"),
        None,
        &[],
        None,
    );
}

#[test]
fn for_over_an_optional_array() {
    expect_error(
        "fn f(m: [i64]?) { for x in m {} }\nfn main() {}",
        "`m` may be `None`",
        (1, 28, "m"),
        None,
        &[],
        Some("unwrap it with `if let`"),
    );
}

#[test]
fn loop_variable_is_immutable() {
    expect_error(
        "fn f(xs: [i64]) {\n    for x in xs {\n        x = 1\n    }\n}\nfn main() {}",
        "cannot assign to `x`",
        (3, 9, "x = 1"),
        None,
        &[(2, 9, "bound by `for` here")],
        None,
    );
}

#[test]
fn loop_variable_scope() {
    errors(
        "fn f(xs: [i64]) {\n    for x in xs {}\n    print(x)\n}\nfn main() {}",
        &[("cannot find `x` in this scope", (3, 11))],
    );
    clean(
        "fn f(xs: [i64]) {\n    let x = 1\n    for x in xs { print(x) }\n    print(x)\n}\nfn main() {}",
    );
    clean(
        "fn f(xs: [i64]) {\n    for x in xs {\n        if x == 1 { continue }\n        if x == 2 { break }\n    }\n}\nfn main() {}",
    );
}

#[test]
fn for_has_no_value() {
    error(
        "fn f(xs: [i64]) -> i64 { for x in xs {} }\nfn main() {}",
        "function `f` must return `i64`, but its body has no value",
        (1, 41),
    );
}

#[test]
fn for_poison() {
    errors(
        "fn main() {\n    for x in nope { print(x + true) }\n}",
        &[("cannot find `nope` in this scope", (2, 14))],
    );
}
