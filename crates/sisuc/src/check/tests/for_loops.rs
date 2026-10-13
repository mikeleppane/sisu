//! `for` over arrays and ranges: the lowering, what the header accepts, and the loop variable.

use super::*;

#[test]
fn lowers_for_over_array() {
    contains(
        "fn f(xs: [i64]) { for x in xs { print(x) } }\nfn main() {}",
        &[
            "(block (let #1 xs#0) (let #2 (len #1)) (let mut #3 0) (loop (block (if (< #3 #2) (block (let x#4 (index #1 #3)) (= #3 (+ #3 1)) (block (print x#4))) (block (break))))))",
        ],
    );
}

#[test]
fn lowers_for_over_range() {
    contains(
        "fn f(n: i64) { for i in 0..n { print(i) } }\nfn main() {}",
        &[
            "(block (let mut #1 0) (let #2 n#0) (loop (block (if (< #1 #2) (block (let i#3 #1) (= #1 (+ #1 1)) (block (print i#3))) (block (break))))))",
        ],
    );
}

#[test]
fn range_bounds_are_i64() {
    codes(
        "fn f(b: u8) { for i in 0..b {} }\nfn main() {}",
        &[(Code::Mismatch, (1, 27))],
    );
}

#[test]
fn range_start_is_i64() {
    codes(
        "fn f(b: u8) { for i in b..0 {} }\nfn main() {}",
        &[(Code::Mismatch, (1, 24))],
    );
}

#[test]
fn loop_variable_has_the_element_type() {
    clean("fn f(xs: [u8]) { for x in xs { let y: u8 = x } }\nfn main() {}");
    codes(
        "fn f(xs: [u8]) { for x in xs { let y: bool = x } }\nfn main() {}",
        &[(Code::Mismatch, (1, 46))],
    );
}

#[test]
fn for_needs_array() {
    diagnostic(
        "fn main() { for x in 5 {} }",
        Code::ForNeedsArray,
        (1, 22, "5"),
        &[],
    );
}

#[test]
fn for_over_an_optional_array() {
    let d = diagnostic(
        "fn f(m: [i64]?) { for x in m {} }\nfn main() {}",
        Code::MayBeNone,
        (1, 28, "m"),
        &[],
    );
    assert!(d.fixes().is_empty(), "{d:?}");
}

#[test]
fn loop_variable_is_immutable() {
    diagnostic(
        "fn f(xs: [i64]) {\n    for x in xs {\n        x = 1\n    }\n}\nfn main() {}",
        Code::AssignToImmutable,
        (3, 9, "x = 1"),
        &[(2, 9)],
    );
}

#[test]
fn loop_variable_ends_with_the_loop() {
    codes(
        "fn f(xs: [i64]) {\n    for x in xs {}\n    print(x)\n}\nfn main() {}",
        &[(Code::BindingNotFound, (3, 11))],
    );
}

/// The outer `x` is `bool`, the loop's is `i64`: each typed use clean pins which one it resolves to.
#[test]
fn loop_variable_shadows_an_outer_binding() {
    clean(
        "fn f(xs: [i64]) {\n    let x = true\n    for x in xs {\n        let a: i64 = x\n    }\n    let b: bool = x\n}\nfn main() {}",
    );
}

#[test]
fn break_and_continue_work_in_for() {
    clean(
        "fn f(xs: [i64]) {\n    for x in xs {\n        if x == 1 { continue }\n        if x == 2 { break }\n    }\n}\nfn main() {}",
    );
}

#[test]
fn for_has_no_value() {
    codes(
        "fn f(xs: [i64]) -> i64 { for x in xs {} }\nfn main() {}",
        &[(Code::BodyHasNoValue, (1, 41))],
    );
}

#[test]
fn for_poison() {
    codes(
        "fn main() {\n    for x in nope { print(x + true) }\n}",
        &[(Code::BindingNotFound, (2, 14))],
    );
}

#[test]
fn bad_range_bound_still_checks_the_body() {
    // `i` is `i64` whatever the bounds are, so the body's own error is real.
    codes(
        "fn main() {\n    for i in nope..3 { print(i + true) }\n}",
        &[(Code::BindingNotFound, (2, 14)), (Code::Mismatch, (2, 34))],
    );
}
