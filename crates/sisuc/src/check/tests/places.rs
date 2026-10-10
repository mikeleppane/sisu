//! Element places: their lowering, the temps of compound operators, the spill, and which roots
//! may be written.

use super::*;

/// `class C` with a `var` array field and a method that writes into it.
const C: &str =
    "class C {\n    var cells: [i64]\n    fn set(self, i: i64) { self.cells[i] = 0 }\n}\n";

#[test]
fn lowers_element_writes() {
    let rows = [
        (
            "fn f() {\n    var xs = [0]\n    xs[0] = 9\n}\nfn main() {}".to_owned(),
            "(= (index xs#0 0) 9)",
        ),
        (
            "fn f(y: i64, x: i64) {\n    var grid = [[false]]\n    grid[y][x] = true\n}\nfn main() {}"
                .to_owned(),
            "(= (index (index grid#2 y#0) x#1) true)",
        ),
        (
            format!("{C}fn main() {{}}"),
            "(= (index (. self#0 cells) i#1) 0)",
        ),
    ];
    for (src, part) in rows {
        contains(&src, &[part]);
    }
}

/// Each index, and a field root's base, is bound once; the root binding is not.
#[test]
fn lowers_compound_element_writes() {
    let rows = [
        (
            "fn f(i: i64) {\n    var xs = [0]\n    xs[i] += 1\n}\nfn main() {}".to_owned(),
            "(block (let #2 i#0) (= (index xs#1 #2) (+ (index xs#1 #2) 1)))",
        ),
        (
            "fn n() -> i64 { 0 }\nfn g() {\n    var grid = [[0]]\n    grid[n()][n()] += 1\n}\nfn main() {}"
                .to_owned(),
            "(block (let #1 (call n)) (let #2 (call n)) (= (index (index grid#0 #1) #2) (+ (index (index grid#0 #1) #2) 1)))",
        ),
        (
            format!(
                "{C}fn make() -> C {{ C(cells: [0]) }}\nfn g() {{\n    make().cells[0] += 1\n}}\nfn main() {{}}"
            ),
            "(block (let #0 (call make)) (let #1 0) (= (index (. #0 cells) #1) (+ (index (. #0 cells) #1) 1)))",
        ),
    ];
    for (src, part) in rows {
        contains(&src, &[part]);
    }
}

/// Distinct calls, so an index bound out of source order changes the lowering.
#[test]
fn binds_indexes_in_source_order() {
    let printed = lowered(
        "fn outer() -> i64 { 0 }\nfn inner() -> i64 { 0 }\nfn g() {\n    var grid = [[0]]\n    grid[outer()][inner()] += 1\n}\nfn main() {}",
    );
    assert!(
        printed.contains(
            "(block (let #1 (call outer)) (let #2 (call inner)) (= (index (index grid#0 #1) #2) (+ (index (index grid#0 #1) #2) 1)))"
        ),
        "{printed}"
    );
    for call in ["(call outer)", "(call inner)"] {
        assert_eq!(printed.matches(call).count(), 1, "{call} in {printed}");
    }
}

/// `a` is counted and the value can exit, so the place's operands are bound before it runs.
#[test]
fn spills_an_element_write() {
    contains(
        "class A {\n    var f: [i64]\n}\nfn g(a: A, i: i64, c: bool) -> i64 {\n    a.f[i] = if c { return 0 } else { 1 }\n    0\n}\nfn main() {}",
        &[
            "(block (let #3 a#0) (let #4 i#1) (let #5 (if c#2 (block (return 0)) (block 1))) (= (index (. #3 f) #4) #5))",
        ],
    );
}

/// An index that can exit makes the element write exit, so the call's counted `make()` spills.
#[test]
fn an_exiting_index_spills_the_enclosing_call() {
    assert_eq!(
        printed_g(
            "fn g(c: bool) -> i64 {\n    var xs = [0]\n    f(make(), if true {\n        xs[if c { return 0 } else { 0 }] = 1\n        0\n    } else { 0 })\n    2\n}"
        ),
        "(fn g ((c#0 bool)) i64 (block (var xs#1 (array 0)) (block (let #2 (call make)) (let #3 (if true (block (= (index xs#1 (if c#0 (block (return 0)) (block 0))) 1) 0) (block 0))) (call f #2 #3)) 2))"
    );
}

#[test]
fn element_type_is_expected_of_the_value() {
    contains(
        "fn f() {\n    var xs: [[i64]] = [[1]]\n    xs[0] = []\n}\nfn main() {}",
        &["(= (index xs#0 0) (array))"],
    );
    contains(
        "fn f(i: i64) {\n    var bytes: [u8] = [0]\n    bytes[i] += 1\n}\nfn main() {}",
        &["(+ (index bytes#1 #2) 1u8)"],
    );
}

/// `let` does not reach into the class objects an array holds.
#[test]
fn field_of_an_element_of_a_let_array() {
    clean("class N {\n    var v: i64\n}\nfn main() {\n    let a = [N(v: 1)]\n    a[0].v = 2\n}");
}

#[test]
fn element_of_a_field_of_a_call() {
    clean(&format!(
        "{C}fn make() -> C {{ C(cells: [0]) }}\nfn main() {{\n    make().cells[0] = 1\n}}"
    ));
}

#[test]
fn element_write_counts_as_reassigning() {
    clean("fn main() {\n    var xs = [0]\n    xs[0] = 1\n}");
}

#[test]
fn element_of_a_let() {
    expect_error(
        "fn main() {\n    let xs = [0]\n    xs[0] = 1\n}",
        "cannot assign to an element of `xs`",
        (3, 5, "xs[0] = 1"),
        None,
        &[(2, 9, "declared with `let` here")],
        Some("declare it with `var`"),
    );
}

#[test]
fn element_of_a_parameter() {
    expect_error(
        "fn f(xs: [i64]) { xs[0] = 1 }\nfn main() {}",
        "cannot assign to an element of `xs`",
        (1, 19, "xs[0] = 1"),
        None,
        &[(1, 6, "declared as a parameter here")],
        Some("copy it into a `var`: `var xs = xs`"),
    );
}

#[test]
fn element_of_an_if_let_binding() {
    expect_error(
        "fn f(m: [i64]?) { if let xs = m { xs[0] = 1 } }\nfn main() {}",
        "cannot assign to an element of `xs`",
        (1, 35, "xs[0] = 1"),
        None,
        &[(1, 26, "bound by `if let` here")],
        None,
    );
}

#[test]
fn element_of_a_for_variable() {
    expect_error(
        "fn f(grid: [[i64]]) { for row in grid { row[0] = 1 } }\nfn main() {}",
        "cannot assign to an element of `row`",
        (1, 41, "row[0] = 1"),
        None,
        &[(1, 27, "bound by `for` here")],
        None,
    );
}

#[test]
fn element_of_a_while_let_binding() {
    expect_error(
        "fn f(m: [i64]?) { while let xs = m { xs[0] = 1 } }\nfn main() {}",
        "cannot assign to an element of `xs`",
        (1, 38, "xs[0] = 1"),
        None,
        &[(1, 29, "bound by `while let` here")],
        None,
    );
}

#[test]
fn element_of_a_let_field() {
    expect_error(
        "class C {\n    let cells: [i64]\n    fn set(self) { self.cells[0] = 1 }\n}\nfn main() {}",
        "cannot assign to an element of `cells`",
        (3, 20, "self.cells[0] = 1"),
        None,
        &[(2, 9, "declared with `let` here")],
        Some("declare it with `var`"),
    );
}

#[test]
fn element_value_of_the_wrong_type() {
    error(
        "fn main() {\n    var xs = [0]\n    xs[0] = true\n}",
        "expected `i64`, found `bool`",
        (3, 13),
    );
}

#[test]
fn element_of_an_optional_array() {
    expect_error(
        "fn main() {\n    var m: [i64]? = None\n    m[0] = 1\n}",
        "`m` may be `None`",
        (3, 5, "m"),
        None,
        &[],
        Some("unwrap it with `if let`"),
    );
}

#[test]
fn element_of_a_non_array() {
    expect_error(
        "fn main() {\n    var n = 1\n    n[0] = 2\n}",
        "cannot index `i64`",
        (3, 5, "n"),
        None,
        &[],
        Some("only arrays can be indexed"),
    );
}

/// An unknown root is reported once, and nothing more about the write.
#[test]
fn element_of_an_unknown_name() {
    errors(
        "fn main() {\n    nope[0] = 1\n}",
        &[("cannot find `nope` in this scope", (2, 5))],
    );
}

#[test]
fn element_index_of_the_wrong_type() {
    expect_error(
        "fn f(b: u8) {\n    var xs = [0]\n    xs[b] = 1\n}\nfn main() {}",
        "expected `i64`, found `u8`",
        (3, 8, "b"),
        None,
        &[],
        Some("convert it: `i64(b)`"),
    );
}
