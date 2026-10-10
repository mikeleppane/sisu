//! Arrays: literals and their element join, `[v; n]`, indexing, `len()`, `sum()`,
//! `contains()`, and the rules that keep arrays values.

use super::*;

const NODE: &str = "class Node {\n    let v: i64\n}\n";

/// `src` after the `Node` class, with an empty `main`.
fn with_node(src: &str) -> String {
    format!("{NODE}{src}\nfn main() {{}}")
}

#[test]
fn element_join() {
    contains(
        "fn main() { let xs = [None, 1] }",
        &["(array None (wrap 1))"],
    );
    contains(
        "fn main() { let g = [[], [1]] }",
        &["(array (array) (array 1))"],
    );
    contains(
        "fn f(b: u8) { let a = [b, 1] }\nfn main() {}",
        &["(array b#0 1u8)"],
    );
    contains(
        "fn f(m: i64?, n: i64) {\n    let a = [m, n]\n    let b = [n, m]\n}\nfn main() {}",
        &["(array m#0 (wrap n#1))", "(array (wrap n#1) m#0)"],
    );
}

#[test]
fn expected_type_flows_into_elements() {
    contains(
        &with_node("fn f(n: Node) { let a: [Node?] = [n, n] }"),
        &["(array (wrap n#0) (wrap n#0))"],
    );
    contains(
        "fn main() { let bytes: [u8?] = [1, None] }",
        &["(array (wrap 1u8) None)"],
    );
}

#[test]
fn fill() {
    contains(
        "fn f(n: i64) { let a = [0; n] }\nfn main() {}",
        &["(array-fill 0 n#0)"],
    );
    contains(
        "fn main() {\n    let b: [u8] = [0; 4]\n    let a: [i64?] = [5; 2]\n}",
        &["(array-fill 0u8 4)", "(array-fill (wrap 5) 2)"],
    );
}

#[test]
fn fill_allows_none_and_arrays_of_classes() {
    clean(&with_node("fn f() { let xs: [Node?] = [None; 3] }"));
    clean(&with_node("fn f(n: Node) { let a = [[n]; 3] }"));
}

#[test]
fn empty_array_typed_by_sibling_branch() {
    contains(
        "fn f(c: bool, ys: [i64]) {\n    let a = if c { [] } else { ys }\n    let b = if c { ys } else { [] }\n}\nfn main() {}",
        &[
            "(let a#2 (if c#0 (block (array)) (block ys#1)))",
            "(let b#3 (if c#0 (block ys#1) (block (array))))",
        ],
    );
    contains(
        "fn f(c: bool, xs: [i64]) -> [i64] { if c { xs } else { [] } }\nfn main() {}",
        &["(block (array))"],
    );
}

#[test]
fn index_and_len() {
    contains(
        "fn f(xs: [[i64]], i: i64) -> i64 { xs[i][0] }\nfn main() {}",
        &["(index (index xs#0 i#1) 0)"],
    );
    contains(
        "fn f(m: [i64]?) -> i64? { m?.len() }\nfn main() {}",
        &["(len #1)"],
    );
}

#[test]
fn equality_wraps_the_plain_side() {
    contains(
        "fn f(a: [i64], b: [i64]?) -> bool { a == b }\nfn main() {}",
        &["(== (wrap a#0) b#1)"],
    );
}

#[test]
fn operands_spill_before_an_exit() {
    contains(
        "fn make() -> [i64] { [1] }\nfn f(c: bool) -> i64 { make()[if c { return 0 } else { 1 }] }\nfn main() {}",
        &[
            "(block (let #1 (call make)) (let #2 (if c#0 (block (return 0)) (block 1))) (index #1 #2))",
        ],
    );
    contains(
        "fn f(c: bool) -> i64 {\n    let g = [[1], if c { return 0 } else { [2] }]\n    0\n}\nfn main() {}",
        &[
            "(block (let #1 (array 1)) (let #2 (if c#0 (block (return 0)) (block (array 2)))) (array #1 #2))",
        ],
    );
    contains(
        "fn f(c: bool) -> i64 {\n    let g = [[1]; if c { return 0 } else { 3 }]\n    0\n}\nfn main() {}",
        &[
            "(block (let #1 (array 1)) (let #2 (if c#0 (block (return 0)) (block 3))) (array-fill #1 #2))",
        ],
    );
}

#[test]
fn empty_array_cannot_be_inferred() {
    expect_error(
        "fn main() { let xs = [] }",
        "cannot infer the type of `[]`",
        (1, 22, "[]"),
        None,
        &[],
        Some("write the type: `let xs: [i64] = []`"),
    );
}

#[test]
fn only_the_first_none_cannot_be_inferred() {
    errors(
        "fn main() { let x = [None, None] }",
        &[("cannot infer the type of `None`", (1, 22))],
    );
}

#[test]
fn empty_array_where_no_array_is_expected() {
    errors(
        "fn main() { let x: i64 = [] }",
        &[("expected `i64`, found `[]`", (1, 26))],
    );
}

#[test]
fn element_does_not_fit() {
    errors(
        "fn main() { let a = [1, true] }",
        &[("expected `i64`, found `bool`", (1, 25))],
    );
    errors(
        "fn main() { print([1] == [true]) }",
        &[("expected `i64`, found `bool`", (1, 27))],
    );
}

#[test]
fn array_of_t_is_not_array_of_optional_t() {
    errors(
        &with_node("fn f(a: [Node]) { let b: [Node?] = a }"),
        &[("expected `[Node?]`, found `[Node]`", (4, 36))],
    );
}

#[test]
fn fill_with_one_object() {
    let help = Some("start from `[None; n]` with type `[Node?]` and fill it in a loop");
    let message = "every element would be the same `Node` object";
    expect_error(
        &format!("{NODE}fn main() {{ let a = [Node(v: 1); 3] }}"),
        message,
        (4, 22, "Node(v: 1)"),
        None,
        &[],
        help,
    );
    expect_error(
        &with_node("fn f(m: Node?) { let a = [m; 3] }"),
        message,
        (4, 27, "m"),
        None,
        &[],
        help,
    );
}

#[test]
fn fill_applies_the_class_rule_before_the_element_type() {
    errors(
        &with_node("fn f(n: Node) { let a: [i64] = [n; 3] }"),
        &[
            ("every element would be the same `Node` object", (4, 33)),
            ("expected `i64`, found `Node`", (4, 33)),
        ],
    );
}

#[test]
fn fill_length_is_i64() {
    errors(
        "fn main() { let a = [0; true] }",
        &[("expected `i64`, found `bool`", (1, 25))],
    );
}

#[test]
fn index_is_i64() {
    errors(
        "fn f(b: u8, xs: [i64]) { print(xs[b]) }\nfn main() {}",
        &[("expected `i64`, found `u8`", (1, 35))],
    );
}

#[test]
fn index_an_optional_array() {
    expect_error(
        "fn f(m: [i64]?) { print(m[0]) }\nfn main() {}",
        "`m` may be `None`",
        (1, 25, "m"),
        None,
        &[],
        Some("unwrap it with `if let`"),
    );
}

#[test]
fn index_a_non_array() {
    expect_error(
        "fn f(n: i64) { print(n[0]) }\nfn main() {}",
        "cannot index `i64`",
        (1, 22, "n"),
        None,
        &[],
        Some("only arrays can be indexed"),
    );
    errors(
        &with_node("fn f(m: Node?) { print(m[0]) }"),
        &[("cannot index `Node?`", (4, 24))],
    );
}

#[test]
fn unknown_members() {
    errors(
        "fn f(xs: [i64]) { xs.push(1) }\nfn main() {}",
        &[("`[i64]` has no method `push`", (1, 22))],
    );
    errors(
        "fn f(xs: [i64]) { print(xs.foo) }\nfn main() {}",
        &[("`[i64]` has no field `foo`", (1, 28))],
    );
}

#[test]
fn len_takes_no_arguments() {
    errors(
        "fn f(xs: [i64]) { print(xs.len(1)) }\nfn main() {}",
        &[("`len` takes 0 arguments, found 1", (1, 25))],
    );
}

#[test]
fn len_is_not_a_field() {
    expect_error(
        "fn f(xs: [i64]) { print(xs.len) }\nfn main() {}",
        "`len` is a method of `[i64]`, not a field",
        (1, 28, "len"),
        None,
        &[],
        Some("call it: `len()`"),
    );
}

#[test]
fn sum_and_contains() {
    contains(
        "fn f(xs: [i64]) -> i64 { xs.sum() }\nfn main() {}",
        &["(sum xs#0)"],
    );
    contains(
        "fn f(xs: [u8?]) -> bool { xs.contains(5) }\nfn main() {}",
        &["(contains xs#0 (wrap 5u8))"],
    );
    contains(
        "fn f(m: [i64]?) -> bool? { m?.contains(1) }\nfn main() {}",
        &["(contains #1 1)"],
    );
}

#[test]
fn contains_spills_for_an_exit_in_its_value() {
    contains(
        "fn f(xs: [[i64]], c: bool) -> bool { xs.contains(if c { return false } else { [1] }) }\nfn main() {}",
        &[
            "(block (let #2 xs#0) (let #3 (if c#1 (block (return false)) (block (array 1)))) (contains #2 #3))",
        ],
    );
}

#[test]
fn sum_needs_an_i64_array() {
    errors(
        "fn f(xs: [bool]) { print(xs.sum()) }\nfn main() {}",
        &[("`sum` needs an `[i64]`, found `[bool]`", (1, 29))],
    );
}

#[test]
fn contains_argument_is_an_element() {
    errors(
        "fn f(xs: [i64]) { print(xs.contains(true)) }\nfn main() {}",
        &[("expected `i64`, found `bool`", (1, 37))],
    );
}

/// A failed `contains` is `Error` when its argument is the problem, so a use of the result
/// adds no error. A poisoned receiver with a good argument still has type `bool`.
#[test]
fn contains_poison() {
    errors(
        "fn f(xs: [i64]) {\n    let b = xs.contains(true)\n    print(b + 1)\n}\nfn main() {}",
        &[("expected `i64`, found `bool`", (2, 25))],
    );
    errors(
        "fn f(xs: [i64]) {\n    let b = xs.contains(v: 1)\n    print(b + 1)\n}\nfn main() {}",
        &[("labels on arguments come in milestone 10", (2, 25))],
    );
    errors(
        "fn f(g: [[i64]]) {\n    let b = g[nope].contains(1)\n    print(b + 1)\n}\nfn main() {}",
        &[
            ("cannot find `nope` in this scope", (2, 15)),
            ("expected `i64`, found `bool`", (3, 11)),
        ],
    );
}

#[test]
fn method_argument_counts() {
    errors(
        "fn f(xs: [i64]) { print(xs.sum(1)) }\nfn main() {}",
        &[("`sum` takes 0 arguments, found 1", (1, 25))],
    );
    errors(
        "fn f(xs: [i64]) { print(xs.contains()) }\nfn main() {}",
        &[("`contains` takes 1 argument, found 0", (1, 25))],
    );
}

#[test]
fn len_of_an_optional_array() {
    expect_error(
        "fn f(m: [i64]?) { print(m.len()) }\nfn main() {}",
        "`m` may be `None`",
        (1, 25, "m"),
        None,
        &[],
        Some("use `?.`, or unwrap it with `if let`"),
    );
}

#[test]
fn is_on_arrays() {
    expect_error(
        "fn f(a: [i64], b: [i64]) { print(a is b) }\nfn main() {}",
        "`is` compares class objects; arrays are values",
        (1, 34, "a"),
        None,
        &[],
        Some("use `==`"),
    );
    expect_error(
        "fn f(a: [i64]?, b: [i64]) { print(a is b) }\nfn main() {}",
        "`is` compares class objects; arrays are values",
        (1, 35, "a"),
        None,
        &[],
        Some("use `==`"),
    );
}

#[test]
fn print_an_array() {
    expect_error(
        "fn f(xs: [i64]) { print(xs) }\nfn main() {}",
        "cannot print `[i64]`",
        (1, 25, "xs"),
        None,
        &[],
        Some("print each element in a `for` loop"),
    );
    expect_error(
        "fn f(m: [i64]?) { print(m) }\nfn main() {}",
        "cannot print `[i64]?`",
        (1, 25, "m"),
        None,
        &[],
        Some("print each element in a `for` loop"),
    );
}

#[test]
fn unknown_element_type() {
    errors(
        "fn f(xs: [Nope]) {}\nfn main() {}",
        &[("unknown type `Nope`", (1, 11))],
    );
}

#[test]
fn unknown_expected_type_is_reported_once() {
    for init in ["[]", "[None]", "[None; 2]"] {
        errors(
            &format!("fn main() {{ let a: Nope = {init} }}"),
            &[("unknown type `Nope`", (1, 20))],
        );
    }
}

#[test]
fn unknown_name_poisons() {
    for (init, col) in [
        ("nope[0]", 13),
        ("[nope, 1]", 14),
        ("[nope, None]", 14),
        ("[nope; 3]", 14),
    ] {
        errors(
            &format!("fn main() {{\n    let x = {init}\n    print(x + true)\n}}"),
            &[("cannot find `nope` in this scope", (2, col))],
        );
    }
}

#[test]
fn own_error_poisons() {
    for (src, message, pos) in [
        (
            "fn main() {\n    let x = [1, []]\n    print(x + true)\n}".to_owned(),
            "expected `i64?`, found `[]`",
            (2, 17),
        ),
        (
            format!("{NODE}fn main() {{\n    let x = [Node(v: 1); 3]\n    print(x + true)\n}}"),
            "every element would be the same `Node` object",
            (5, 14),
        ),
        (
            "fn f(xs: [i64], b: u8) {\n    let x = xs[b]\n    print(x + true)\n}\nfn main() {}"
                .to_owned(),
            "expected `i64`, found `u8`",
            (2, 16),
        ),
        (
            "fn main() {\n    let x: [i64] = [true]\n    print(x + true)\n}".to_owned(),
            "expected `i64`, found `bool`",
            (2, 21),
        ),
    ] {
        errors(&src, &[(message, pos)]);
    }
}
