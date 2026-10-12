use super::*;
use crate::diagnostic::line_col;
use crate::lexer::lex;
use crate::parser::parse;

mod arrays;
mod for_loops;
mod places;
mod u8;

fn checked(src: &str) -> (Option<tir::Program>, Vec<Diagnostic>) {
    check(&parse(&lex(src).expect("source lexes")).expect("source parses"))
}

fn diags(src: &str) -> Vec<Diagnostic> {
    checked(src).1
}

/// The typed tree of a program that checks without a diagnostic.
fn lowered(src: &str) -> String {
    let (program, ds) = checked(src);
    assert!(ds.is_empty(), "{ds:?}");
    program.expect("no errors").to_string()
}

/// Asserts that `src` checks clean and its `tir` contains each of `parts`.
fn contains(src: &str, parts: &[&str]) {
    let printed = lowered(src);
    for part in parts {
        assert!(printed.contains(part), "{part} in {printed}");
    }
}

/// Asserts the diagnostics are exactly these errors, in order: `(message, (line, col))`,
/// and that no `tir` comes out.
fn errors(src: &str, expected: &[(&str, (usize, usize))]) {
    let (program, ds) = checked(src);
    assert!(program.is_none(), "an error yields no `tir`");
    let found: Vec<_> = ds
        .iter()
        .map(|d| (d.severity, d.message.as_str(), at(src, d.span)))
        .collect();
    let expected: Vec<_> = expected
        .iter()
        .map(|&(message, pos)| (Severity::Error, message, pos))
        .collect();
    assert_eq!(found, expected);
}

fn at(src: &str, span: Span) -> (usize, usize) {
    line_col(src, span.start)
}

/// Asserts exactly one error and no `tir`, and returns it.
fn single_error(src: &str, message: &str) -> Diagnostic {
    let (program, mut ds) = checked(src);
    assert!(program.is_none(), "an error yields no `tir`");
    assert_eq!(ds.len(), 1, "{ds:?}");
    let d = ds.remove(0);
    assert_eq!(d.severity, Severity::Error);
    assert_eq!(d.message, message);
    d
}

/// Asserts exactly one error with these parts, and no `tir`; `pos` is `(line, col, text)`
/// of the whole primary span, `secondary` is `(line, col, text)` rows.
fn expect_error(
    src: &str,
    message: &str,
    pos: (usize, usize, &str),
    label: Option<&str>,
    secondary: &[(usize, usize, &str)],
    help: Option<&str>,
) {
    let d = single_error(src, message);
    assert_eq!(
        (at(src, d.span), &src[d.span.start..d.span.end]),
        ((pos.0, pos.1), pos.2)
    );
    assert_eq!(d.label.as_deref(), label);
    let found: Vec<_> = d
        .secondary
        .iter()
        .map(|(span, text)| {
            let (line, col) = at(src, *span);
            (line, col, text.as_str())
        })
        .collect();
    assert_eq!(found, secondary);
    assert_eq!(d.help.as_deref(), help);
}

/// Asserts exactly one error with this message and position, and no label or help.
fn error(src: &str, message: &str, pos: (usize, usize)) {
    let d = single_error(src, message);
    assert_eq!(at(src, d.span), pos);
    assert_eq!(d.label, None);
    assert!(d.secondary.is_empty(), "{:?}", d.secondary);
    assert_eq!(d.help, None);
}

/// Asserts exactly one error: a `never` expression used as a value, at `pos`.
fn never_value(src: &str, pos: (usize, usize)) {
    let d = single_error(src, "this expression never produces a value");
    assert_eq!(at(src, d.span), pos);
    assert_eq!(
        d.help.as_deref(),
        Some("every path through it returns, breaks or continues")
    );
    assert_eq!(d.label, None);
    assert!(d.secondary.is_empty(), "{:?}", d.secondary);
}

/// Asserts exactly one diagnostic: a warning with this message and position.
fn warning(src: &str, message: &str, pos: (usize, usize)) {
    let ds = diags(src);
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].severity, Severity::Warning);
    assert_eq!(ds[0].message, message);
    assert_eq!(at(src, ds[0].span), pos);
}

fn clean(src: &str) {
    let ds = diags(src);
    assert!(ds.is_empty(), "{ds:?}");
}

#[test]
fn duplicate_function() {
    expect_error(
        "fn main() {}\nfn main() {}",
        "function `main` is defined twice",
        (2, 4, "main"),
        None,
        &[(1, 4, "first defined here")],
        None,
    );
}

#[test]
fn print_is_built_in() {
    expect_error(
        "fn print() {}\nfn main() {}",
        "`print` is a built-in function",
        (1, 4, "print"),
        None,
        &[],
        None,
    );
}

#[test]
fn missing_main() {
    let src = "fn f() {}";
    expect_error(src, "no `main` function", (1, 1, ""), None, &[], None);
    assert_eq!(diags(src)[0].span, Span::new(0, 0));
}

#[test]
fn main_with_params() {
    expect_error(
        "fn main(x: i64) {}",
        "`main` takes no parameters and returns no value",
        (1, 4, "main"),
        None,
        &[],
        None,
    );
}

#[test]
fn main_with_return_type() {
    expect_error(
        "fn main() -> i64 { 0 }",
        "`main` takes no parameters and returns no value",
        (1, 4, "main"),
        None,
        &[],
        None,
    );
}

#[test]
fn unknown_type() {
    expect_error(
        "fn main() { let x: str = 1 }",
        "unknown type `str`",
        (1, 20, "str"),
        None,
        &[],
        Some("the types are `i64`, `u8`, `bool`, classes and arrays such as `[i64]`"),
    );
}

#[test]
fn unknown_name() {
    expect_error(
        "fn main() { print(y) }",
        "cannot find `y` in this scope",
        (1, 19, "y"),
        None,
        &[],
        None,
    );
}

#[test]
fn unknown_function() {
    expect_error(
        "fn main() { g() }",
        "cannot find function or class `g`",
        (1, 13, "g"),
        None,
        &[],
        None,
    );
}

#[test]
fn redeclare_in_block() {
    expect_error(
        "fn main() {\n    let x = 1\n    let x = 2\n}",
        "`x` is already declared in this scope",
        (3, 9, "x"),
        None,
        &[(2, 9, "first declared here")],
        None,
    );
}

#[test]
fn repeated_param() {
    expect_error(
        "fn f(a: i64, a: i64) {}\nfn main() {}",
        "`a` is already declared in this scope",
        (1, 14, "a"),
        None,
        &[(1, 6, "first declared here")],
        None,
    );
}

#[test]
fn shadowing_in_inner_block() {
    let src = "fn main() {\n    let x = 1\n    if true {\n        let x = true\n        print(x)\n    }\n    print(x)\n}";
    assert!(diags(src).is_empty());
}

#[test]
fn functions_in_any_order() {
    assert!(diags("fn main() { print(f(1)) }\nfn f(n: i64) -> i64 { n }").is_empty());
}

#[test]
fn variable_may_shadow_a_param() {
    let src = "fn f(n: i64) {\n    let mut n = n\n    n = 1\n}\nfn main() {}";
    clean(src);
}

#[test]
fn every_error_in_one_run() {
    errors(
        "fn main() {\n    print(a)\n    let b: bool = 1\n    print(1 + true)\n}",
        &[
            ("cannot find `a` in this scope", (2, 11)),
            ("expected `bool`, found `i64`", (3, 19)),
            ("expected `i64`, found `bool`", (4, 15)),
        ],
    );
}

#[test]
fn poisoned_binding_reports_once() {
    errors(
        "fn main() {\n    let x = nope\n    print(x + 1)\n}",
        &[("cannot find `nope` in this scope", (2, 13))],
    );
}

#[test]
fn poisoned_operand_checks_siblings() {
    errors(
        "fn f(a: i64, b: i64) {}\nfn main() {\n    f(nope, 1 + true)\n}",
        &[
            ("cannot find `nope` in this scope", (3, 7)),
            ("expected `i64`, found `bool`", (3, 17)),
        ],
    );
}

#[test]
fn if_with_failed_branch_takes_other_type() {
    errors(
        "fn main() {\n    let y = if true { nope } else { 1 }\n    print(y + true)\n}",
        &[
            ("cannot find `nope` in this scope", (2, 23)),
            ("expected `i64`, found `bool`", (3, 15)),
        ],
    );
}

#[test]
fn unknown_let_type_poisons() {
    errors(
        "fn main() {\n    let x: str = 1\n    print(x + true)\n}",
        &[("unknown type `str`", (2, 12))],
    );
}

#[test]
fn unknown_param_type_still_checks_body() {
    errors(
        "fn f(a: str) -> i64 { true }\nfn main() { f(1) }",
        &[
            ("unknown type `str`", (1, 9)),
            ("expected `i64`, found `bool`", (1, 23)),
        ],
    );
}

#[test]
fn duplicate_checked_against_own_signature() {
    errors(
        "fn f(a: i64) {}\nfn f(b: bool) -> bool { b }\nfn main() {}",
        &[("function `f` is defined twice", (2, 4))],
    );
}

#[test]
fn duplicate_call_resolves_to_first() {
    errors(
        "fn f(a: i64) -> i64 { a }\nfn f(b: bool) -> bool { b }\nfn main() {\n    print(f(1))\n}",
        &[("function `f` is defined twice", (2, 4))],
    );
}

#[test]
fn duplicate_let_keeps_first() {
    errors(
        "fn main() {\n    let x = 1\n    let x = true\n    print(x + 1)\n}",
        &[("`x` is already declared in this scope", (3, 9))],
    );
}

#[test]
fn failed_call_still_checks_arguments() {
    const MISMATCH: &str = "expected `i64`, found `bool`";
    let rows = [
        (
            "unknown function",
            "fn main() {\n    g(1 + true)\n}",
            vec![
                ("cannot find function or class `g`", (2, 5)),
                (MISMATCH, (2, 11)),
            ],
        ),
        (
            "arity",
            "fn f(a: i64) {}\nfn main() {\n    f(1 + true, 2)\n}",
            vec![
                ("`f` takes 1 argument, found 2", (3, 5)),
                (MISMATCH, (3, 11)),
            ],
        ),
        (
            "print arity",
            "fn main() {\n    print(1 + true, 2)\n}",
            vec![
                ("`print` takes one `i64`, `u8` or `bool`", (2, 5)),
                (MISMATCH, (2, 15)),
            ],
        ),
    ];
    for (name, src, expected) in rows {
        println!("row: {name}");
        errors(src, &expected);
    }
}

#[test]
fn duplicate_body_still_checked() {
    errors(
        "fn f() {}\nfn f() { print(nope) }\nfn main() {}",
        &[
            ("function `f` is defined twice", (2, 4)),
            ("cannot find `nope` in this scope", (2, 16)),
        ],
    );
}

#[test]
fn missing_main_and_body_error() {
    errors(
        "fn f() { print(nope) }",
        &[
            ("no `main` function", (1, 1)),
            ("cannot find `nope` in this scope", (1, 16)),
        ],
    );
}

#[test]
fn duplicate_body_uses_own_return_type() {
    errors(
        "fn f() -> i64 { 1 }\nfn f() -> bool { true }\nfn main() {}",
        &[("function `f` is defined twice", (2, 4))],
    );
}

#[test]
fn if_with_failed_and_never_branches_is_error() {
    errors(
        "fn main() {\n    let c = true\n    let x = if c { nope } else { return }\n    print(1)\n}",
        &[("cannot find `nope` in this scope", (3, 20))],
    );
}

/// A rule that fails makes its expression `Error`. Each row binds the failed expression
/// and uses it where a typed result would cascade into a second error.
#[test]
fn failed_rule_makes_its_expression_error() {
    const MISMATCH: &str = "expected `i64`, found `bool`";
    const F: &str = "fn f(a: i64) -> i64 { a }\n";
    let rows: [(&str, String, &str, (usize, usize)); 12] = [
        (
            "unary",
            "fn main() {\n    let x = -true\n    print(x == false)\n}".into(),
            MISMATCH,
            (2, 14),
        ),
        (
            "binary lhs",
            "fn main() {\n    let x = true + 1\n    print(x == false)\n}".into(),
            MISMATCH,
            (2, 13),
        ),
        (
            "binary rhs",
            "fn main() {\n    let x = 1 + true\n    print(x == false)\n}".into(),
            MISMATCH,
            (2, 17),
        ),
        (
            "compare",
            "fn main() {\n    let x = 1 < true < 2\n    print(x + 1)\n}".into(),
            MISMATCH,
            (2, 17),
        ),
        (
            "equal",
            "fn main() {\n    let x = 1 == true\n    print(x + 1)\n}".into(),
            "cannot compare `i64` with `bool`",
            (2, 13),
        ),
        (
            "call arity",
            format!("{F}fn main() {{\n    let x = f()\n    print(x == false)\n}}"),
            "`f` takes 1 argument, found 0",
            (3, 13),
        ),
        (
            "call argument",
            format!("{F}fn main() {{\n    let x = f(true)\n    print(x == false)\n}}"),
            MISMATCH,
            (3, 15),
        ),
        (
            "print arity",
            "fn main() {\n    let x = print(1, 2)\n    print(x)\n}".into(),
            "`print` takes one `i64`, `u8` or `bool`",
            (2, 13),
        ),
        (
            "print unit argument",
            "fn main() {\n    let x = print(print(1))\n    print(x)\n}".into(),
            "expression has no value",
            (2, 19),
        ),
        (
            "if branches",
            "fn main() {\n    let x = if true { 1 } else { false }\n    print(x == false)\n}"
                .into(),
            MISMATCH,
            (2, 34),
        ),
        (
            "if condition with else",
            "fn main() {\n    let x = if 1 { 2 } else { 3 }\n    print(x == false)\n}".into(),
            "expected `bool`, found `i64`",
            (2, 16),
        ),
        (
            "if condition without else",
            "fn main() {\n    let x = if 1 { print(2) }\n    print(x)\n}".into(),
            "expected `bool`, found `i64`",
            (2, 16),
        ),
    ];
    for (name, src, message, pos) in rows {
        println!("row: {name}");
        errors(&src, &[(message, pos)]);
    }
}

#[test]
fn warning_only_program_lowers() {
    let src = "fn main() {\n    let mut x = 1\n    print(x)\n}";
    let (program, ds) = checked(src);
    assert!(program.is_some());
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].severity, Severity::Warning);
    assert_eq!(ds[0].message, "`x` is never reassigned");
    assert_eq!(at(src, ds[0].span), (2, 13));
}

#[test]
fn warning_and_error_in_position_order() {
    // The warning is found when the scope closes, after the error.
    let src = "fn main() {\n    let mut x = 1\n    print(nope)\n}";
    let found: Vec<_> = diags(src)
        .iter()
        .map(|d| (d.severity, d.message.clone(), at(src, d.span)))
        .collect();
    assert_eq!(
        found,
        [
            (
                Severity::Warning,
                "`x` is never reassigned".to_string(),
                (2, 13)
            ),
            (
                Severity::Error,
                "cannot find `nope` in this scope".to_string(),
                (3, 11)
            ),
        ]
    );
}

#[test]
fn trailing_expression_is_the_block_value() {
    let (program, ds) =
        checked("fn f() -> i64 {\n    let x = 1\n    x\n}\nfn main() {\n    let y = f()\n}");
    assert!(ds.is_empty(), "{ds:?}");
    let program = program.expect("no errors");
    let f = &program.functions[0].body;
    assert!(matches!(f.stmts.as_slice(), [tir::Stmt::Let { .. }]));
    assert!(matches!(
        f.value.as_deref(),
        Some(tir::Expr {
            kind: tir::ExprKind::Local(_),
            ..
        })
    ));
    let main = &program.functions[1].body;
    assert!(matches!(main.stmts.as_slice(), [tir::Stmt::Let { .. }]));
    assert!(main.value.is_none());
}

/// A `while` lowers to an expression statement, but it is not an expression in `ast`, so
/// a block that ends in one has no value.
#[test]
fn loop_tail_is_not_the_block_value() {
    let (program, ds) =
        checked("fn main() {\n    let mut i = 0\n    while i < 3 {\n        i += 1\n    }\n}");
    assert!(ds.is_empty(), "{ds:?}");
    let main = &program.expect("no errors").functions[0].body;
    assert!(matches!(
        main.stmts.as_slice(),
        [tir::Stmt::Let { .. }, tir::Stmt::Expr(_)]
    ));
    assert!(main.value.is_none());
}

#[test]
fn lowers_while_to_loop() {
    assert_eq!(
        lowered("fn main() {\n    let mut i = 0\n    while i < 3 {\n        i = i + 1\n    }\n}"),
        "(fn main () unit (block (let mut i#0 0) (loop (block (if (< i#0 3) (block (= i#0 (+ i#0 1))) (block (break)))))))"
    );
}

#[test]
fn lowers_continue() {
    assert_eq!(
        lowered(
            "fn main() {\n    let mut i = 0\n    while i < 3 {\n        i = i + 1\n        if i == 2 { continue }\n        print(i)\n    }\n}"
        ),
        "(fn main () unit (block (let mut i#0 0) (loop (block (if (< i#0 3) (block (= i#0 (+ i#0 1)) (if (== i#0 2) (block (continue))) (print i#0)) (block (break)))))))"
    );
}

#[test]
fn break_outside_loop() {
    errors(
        "fn main() { break }",
        &[("`break` outside a loop", (1, 13))],
    );
}

#[test]
fn continue_outside_loop() {
    errors(
        "fn main() { continue }",
        &[("`continue` outside a loop", (1, 13))],
    );
}

#[test]
fn break_after_loop() {
    errors(
        "fn main() {\n    while true {}\n    break\n}",
        &[("`break` outside a loop", (3, 5))],
    );
}

#[test]
fn break_after_while_let() {
    errors(
        "fn f(n: i64?) {\n    while let x = n {}\n    break\n}\nfn main() {}",
        &[("`break` outside a loop", (3, 5))],
    );
}

#[test]
fn statement_after_break() {
    warning(
        "fn main() {\n    while true {\n        break\n        print(1)\n    }\n}",
        "unreachable code",
        (4, 9),
    );
}

#[test]
fn shadowing_makes_two_locals() {
    assert_eq!(
        lowered(
            "fn main() {\n    let x = 1\n    if true {\n        let x = true\n        print(x)\n    }\n    print(x)\n}"
        ),
        "(fn main () unit (block (let x#0 1) (if true (block (let x#1 true) (print x#1))) (print x#0)))"
    );
}

#[test]
fn lowers_compound_assign() {
    assert_eq!(
        lowered("fn main() {\n    let mut x = 1\n    x *= 3\n    print(x)\n}"),
        "(fn main () unit (block (let mut x#0 1) (= x#0 (* x#0 3)) (print x#0)))"
    );
}

#[test]
fn lowers_not_equal() {
    assert_eq!(
        lowered("fn f(a: i64) -> bool { a != 1 }\nfn main() {}"),
        "(fn f ((a#0 i64)) bool (block (!= a#0 1)))\n(fn main () unit (block))"
    );
}

#[test]
fn unknown_type_in_signature() {
    expect_error(
        "fn f(a: str) {}\nfn main() {}",
        "unknown type `str`",
        (1, 9, "str"),
        None,
        &[],
        Some("the types are `i64`, `u8`, `bool`, classes and arrays such as `[i64]`"),
    );
}

#[test]
fn condition_must_be_bool() {
    error(
        "fn main() { if 1 { } }",
        "expected `bool`, found `i64`",
        (1, 16),
    );
}

#[test]
fn while_condition_must_be_bool() {
    error(
        "fn main() { while 1 { } }",
        "expected `bool`, found `i64`",
        (1, 19),
    );
}

#[test]
fn arithmetic_needs_i64() {
    error(
        "fn main() { print(1 + true) }",
        "expected `i64`, found `bool`",
        (1, 23),
    );
}

#[test]
fn not_needs_bool() {
    error(
        "fn main() { print(!1) }",
        "expected `bool`, found `i64`",
        (1, 20),
    );
}

#[test]
fn equality_needs_same_type() {
    error(
        "fn main() { print(1 == true) }",
        "cannot compare `i64` with `bool`",
        (1, 19),
    );
}

#[test]
fn arity() {
    error(
        "fn f(a: i64) {}\nfn main() { f() }",
        "`f` takes 1 argument, found 0",
        (2, 13),
    );
    error(
        "fn f(a: i64, b: i64) {}\nfn main() { f(1) }",
        "`f` takes 2 arguments, found 1",
        (2, 13),
    );
}

#[test]
fn argument_type() {
    error(
        "fn f(a: i64) {}\nfn main() { f(true) }",
        "expected `i64`, found `bool`",
        (2, 15),
    );
}

#[test]
fn print_no_argument() {
    error(
        "fn main() { print() }",
        "`print` takes one `i64`, `u8` or `bool`",
        (1, 13),
    );
}

#[test]
fn print_two_arguments() {
    error(
        "fn main() { print(1, 2) }",
        "`print` takes one `i64`, `u8` or `bool`",
        (1, 13),
    );
}

#[test]
fn print_unit_argument() {
    error(
        "fn g() {}\nfn main() { print(g()) }",
        "expression has no value",
        (2, 19),
    );
}

#[test]
fn bind_unit() {
    error(
        "fn g() {}\nfn main() { let x = g() }",
        "expression has no value",
        (2, 21),
    );
}

#[test]
fn if_without_else_has_no_value() {
    error(
        "fn main() {\n    let x = if true { 1 }\n}",
        "expression has no value",
        (2, 13),
    );
}

#[test]
fn if_branches_differ() {
    error(
        "fn main() {\n    let x = if true { 1 } else { false }\n}",
        "expected `i64`, found `bool`",
        (2, 34),
    );
}

#[test]
fn body_value_type() {
    error(
        "fn f() -> i64 {\n    true\n}\nfn main() {}",
        "expected `i64`, found `bool`",
        (2, 5),
    );
}

#[test]
fn body_without_value() {
    let src = "fn f() -> i64 {\n    let x = 1\n}\nfn main() {}";
    error(
        src,
        "function `f` must return `i64`, but its body has no value",
        (3, 1),
    );
    assert_eq!(diags(src)[0].span, Span::new(30, 31));
}

#[test]
fn return_value_type() {
    error(
        "fn f() -> i64 {\n    return true\n}\nfn main() {}",
        "expected `i64`, found `bool`",
        (2, 12),
    );
}

#[test]
fn return_needs_value() {
    error(
        "fn f() -> i64 {\n    return\n}\nfn main() {}",
        "`return` needs a value of type `i64`",
        (2, 5),
    );
}

#[test]
fn return_value_in_unit_fn() {
    error(
        "fn main() {\n    return 1\n}",
        "this function returns no value",
        (2, 12),
    );
}

#[test]
fn all_paths_return_is_fine() {
    clean(
        "fn sign(n: i64) -> i64 {\n    if n < 0 { return -1 } else { return 1 }\n}\nfn main() {}",
    );
}

#[test]
fn never_as_return_value() {
    never_value(
        "fn f(c: bool) -> i64 {\n    return if c { return 1 } else { return 2 }\n}\nfn main() {}",
        (2, 12),
    );
}

#[test]
fn annotated_let_type() {
    error(
        "fn main() { let x: bool = 1 }",
        "expected `bool`, found `i64`",
        (1, 27),
    );
}

#[test]
fn annotated_let_matches() {
    clean("fn main() {\n    let x: bool = true\n    print(x)\n}");
}

#[test]
fn never_as_initializer() {
    never_value(
        "fn f(c: bool) -> i64 {\n    let x = if c { return 1 } else { return 2 }\n    x\n}\nfn main() {}",
        (2, 13),
    );
}

#[test]
fn never_as_condition() {
    never_value(
        "fn f(c: bool) -> i64 {\n    while if c { return 1 } else { return 2 } {}\n    1\n}\nfn main() {}",
        (2, 11),
    );
}

#[test]
fn never_as_operand() {
    never_value(
        "fn f(c: bool) -> i64 {\n    1 + if c { return 1 } else { return 2 }\n}\nfn main() {}",
        (2, 9),
    );
}

#[test]
fn statement_after_return() {
    warning(
        "fn main() {\n    return\n    print(1)\n}",
        "unreachable code",
        (3, 5),
    );
}

#[test]
fn block_after_return_is_never() {
    warning(
        "fn f() -> i64 {\n    return 1\n    print(2)\n}\nfn main() {}",
        "unreachable code",
        (3, 5),
    );
}

#[test]
fn neg_needs_i64() {
    error(
        "fn main() { print(-true) }",
        "expected `i64`, found `bool`",
        (1, 20),
    );
}

#[test]
fn less_needs_i64() {
    error(
        "fn main() { print(true < false) }",
        "expected `i64`, found `bool`",
        (1, 19),
    );
}

#[test]
fn and_needs_bool() {
    error(
        "fn main() { print(1 && true) }",
        "expected `bool`, found `i64`",
        (1, 19),
    );
}

#[test]
fn unit_operand() {
    error(
        "fn g() {}\nfn main() { print(1 + g()) }",
        "expression has no value",
        (2, 23),
    );
}

#[test]
fn unit_argument() {
    error(
        "fn g() {}\nfn f(a: i64) {}\nfn main() { f(g()) }",
        "expression has no value",
        (3, 15),
    );
}

#[test]
fn separate_namespaces() {
    clean("fn f() -> i64 { 1 }\nfn main() {\n    let f = 2\n    print(f)\n    print(f())\n}");
}

#[test]
fn never_branch_takes_other_type() {
    clean(
        "fn f(c: bool) -> i64 {\n    let x = if c { return 1 } else { 2 }\n    let y = if c { 3 } else { return 4 }\n    x + y\n}\nfn main() {}",
    );
}

#[test]
fn unreachable_warned_once_per_block() {
    warning(
        "fn main() {\n    return\n    print(1)\n    print(2)\n}",
        "unreachable code",
        (3, 5),
    );
}

#[test]
fn statements_after_return_are_checked() {
    let src = "fn main() {\n    return\n    print(true + 1)\n}";
    let ds = diags(src);
    let found: Vec<_> = ds
        .iter()
        .map(|d| (d.severity, d.message.as_str(), at(src, d.span)))
        .collect();
    assert_eq!(
        found,
        [
            (Severity::Warning, "unreachable code", (3, 5)),
            (Severity::Error, "expected `i64`, found `bool`", (3, 11)),
        ]
    );
}

#[test]
fn assign_to_let() {
    let src = "fn main() {\n    let n = 0\n    n = 1\n}";
    expect_error(
        src,
        "cannot assign to `n`",
        (3, 5, "n = 1"),
        Some("cannot assign twice"),
        &[(2, 9, "declared with `let` here")],
        Some("declare it with `let mut`"),
    );
    let span = diags(src)[0].span;
    assert_eq!(&src[span.start..span.end], "n = 1");
}

#[test]
fn never_assigned_value() {
    never_value(
        "fn f(c: bool) -> i64 {\n    let mut x = 0\n    x = if c { return 1 } else { return 2 }\n    x\n}\nfn main() {}",
        (3, 9),
    );
}

#[test]
fn compound_assign_to_let() {
    expect_error(
        "fn main() {\n    let n = 0\n    n += 1\n}",
        "cannot assign to `n`",
        (3, 5, "n += 1"),
        Some("cannot assign twice"),
        &[(2, 9, "declared with `let` here")],
        Some("declare it with `let mut`"),
    );
}

#[test]
fn assign_to_param() {
    expect_error(
        "fn f(n: i64) {\n    n = 1\n}\nfn main() {}",
        "cannot assign to `n`",
        (2, 5, "n = 1"),
        None,
        &[(1, 6, "declared as a parameter here")],
        Some("copy it into a variable: `let mut n = n`"),
    );
}

#[test]
fn compound_assign_to_param() {
    expect_error(
        "fn f(n: i64) {\n    n += 1\n}\nfn main() {}",
        "cannot assign to `n`",
        (2, 5, "n += 1"),
        None,
        &[(1, 6, "declared as a parameter here")],
        Some("copy it into a variable: `let mut n = n`"),
    );
}

#[test]
fn compound_assign_unknown_target_once() {
    error(
        "fn main() {\n    nope += 1\n}",
        "cannot find `nope` in this scope",
        (2, 5),
    );
}

#[test]
fn compound_assign_unknown_target_and_bad_value() {
    errors(
        "fn main() {\n    nope += true + 1\n}",
        &[
            ("cannot find `nope` in this scope", (2, 5)),
            ("expected `i64`, found `bool`", (2, 13)),
        ],
    );
}

#[test]
fn assign_type() {
    error(
        "fn main() {\n    let mut x = 1\n    x = true\n}",
        "expected `i64`, found `bool`",
        (3, 9),
    );
}

#[test]
fn unused_variable() {
    let src = "fn main() {\n    let mut count = 0\n    print(count)\n}";
    warning(src, "`count` is never reassigned", (2, 13));
    assert_eq!(diags(src)[0].help.as_deref(), Some("declare it with `let`"));
}

#[test]
fn unused_variables_in_source_order() {
    // The inner scope pops first, so this fails unless warnings are sorted.
    let src = "fn main() {\n    let mut a = 0\n    if true {\n        let mut b = 0\n        print(b)\n    }\n    print(a)\n}";
    let found: Vec<_> = diags(src)
        .iter()
        .map(|d| (d.severity, d.message.clone(), at(src, d.span)))
        .collect();
    assert_eq!(
        found,
        [
            (
                Severity::Warning,
                "`a` is never reassigned".to_string(),
                (2, 13)
            ),
            (
                Severity::Warning,
                "`b` is never reassigned".to_string(),
                (4, 17)
            ),
        ]
    );
}

#[test]
fn variable_reassigned_in_inner_block() {
    clean("fn main() {\n    let mut n = 0\n    if true { n = 1 }\n    print(n)\n}");
}

#[test]
fn empty_file_has_no_main() {
    let ds = diags("");
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].message, "no `main` function");
    assert_eq!(at("", ds[0].span), (1, 1));
    assert!(
        ds[0]
            .render("empty.sisu", "")
            .starts_with("error: no `main` function")
    );
}

#[test]
fn class_and_function_share_names() {
    expect_error(
        "class A {}\nfn A() {}\nfn main() {}",
        "`A` is already a class",
        (2, 4, "A"),
        None,
        &[(1, 7, "first defined here")],
        None,
    );
}

#[test]
fn class_after_function_of_same_name() {
    expect_error(
        "fn A() {}\nclass A {}\nfn main() {}",
        "`A` is already a function",
        (2, 7, "A"),
        None,
        &[(1, 4, "first defined here")],
        None,
    );
}

#[test]
fn class_defined_twice() {
    // The `bool` argument shows that `X` still names the first class.
    let src = "class X {\n    a: i64\n}\nclass X {\n    a: bool\n}\nfn main() { X(a: true) }";
    errors(
        src,
        &[
            ("class `X` is defined twice", (4, 7)),
            ("expected `i64`, found `bool`", (7, 18)),
        ],
    );
    let secondary: Vec<_> = diags(src)[0]
        .secondary
        .iter()
        .map(|(span, text)| (at(src, *span), text.clone()))
        .collect();
    assert_eq!(secondary, [((1, 7), "first defined here".to_owned())]);
}

#[test]
fn class_named_print() {
    expect_error(
        "class print {}\nfn main() {}",
        "`print` is a built-in function",
        (1, 7, "print"),
        None,
        &[],
        None,
    );
}

#[test]
fn duplicate_member() {
    expect_error(
        "class A {\n    x: i64\n    fn x(self) {}\n}\nfn main() {}",
        "`x` is already a field of `A`",
        (3, 8, "x"),
        None,
        &[(2, 5, "first defined here")],
        None,
    );
}

#[test]
fn duplicate_field() {
    expect_error(
        "class A {\n    fn x(self) {}\n    mut x: i64\n}\nfn main() {}",
        "`x` is already a method of `A`",
        (3, 9, "x"),
        None,
        &[(2, 8, "first defined here")],
        None,
    );
}

#[test]
fn builtin_type_as_class() {
    error(
        "class i64 {}\nfn main() {}",
        "`i64` is a built-in type",
        (1, 7),
    );
}

#[test]
fn never_constructed_self() {
    expect_error(
        "class Node {\n    next: Node\n}\nfn main() {}",
        "`Node` can never be constructed: field `next` needs a `Node`",
        (2, 5, "next"),
        None,
        &[],
        Some("make it optional: `Node?`"),
    );
}

#[test]
fn never_constructed_cycle_of_two() {
    expect_error(
        "class A {\n    b: B\n}\nclass B {\n    a: A\n}\nfn main() {}",
        "`A` can never be constructed: field `b` needs a `B`",
        (2, 5, "b"),
        None,
        &[],
        Some("make it optional: `B?`"),
    );
}

#[test]
fn never_constructed_once_per_strongly_connected_component() {
    // `A`, `B` and `C` form one component of two cycles, `E` another; `D` only points in.
    errors(
        "class A {\n    b: B\n    c: C\n}\nclass B {\n    a: A\n}\nclass C {\n    a: A\n}\nclass D {\n    a: A\n}\nclass E {\n    e: E\n}\nfn main() {}",
        &[
            (
                "`A` can never be constructed: field `b` needs a `B`",
                (2, 5),
            ),
            (
                "`E` can never be constructed: field `e` needs a `E`",
                (15, 5),
            ),
        ],
    );
}

/// A class that only points into a cycle, declared first, does not hide the cycle.
#[test]
fn never_constructed_after_a_class_that_points_in() {
    expect_error(
        "class D {\n    a: A\n}\nclass A {\n    next: A\n}\nfn main() {}",
        "`A` can never be constructed: field `next` needs a `A`",
        (5, 5, "next"),
        None,
        &[],
        Some("make it optional: `A?`"),
    );
}

/// The field reported is the one into the cycle, not the first field of a class type.
#[test]
fn never_constructed_names_the_field_into_the_cycle() {
    expect_error(
        "class Tag {}\nclass A {\n    tag: Tag\n    next: A\n}\nfn main() {}",
        "`A` can never be constructed: field `next` needs a `A`",
        (4, 5, "next"),
        None,
        &[],
        Some("make it optional: `A?`"),
    );
}

#[test]
fn constructor_missing_label() {
    expect_error(
        "class P {\n    x: i64\n}\nfn main() { P(1) }",
        "this argument needs a label",
        (4, 15, "1"),
        None,
        &[],
        Some("label it with its field: `x: ...`"),
    );
}

#[test]
fn constructor_unknown_label() {
    error(
        "class P {\n    x: i64\n}\nfn main() { P(z: 1) }",
        "`P` has no field `z`",
        (4, 15),
    );
}

#[test]
fn constructor_out_of_order() {
    expect_error(
        "class P {\n    x: i64\n    y: i64\n}\nfn main() { P(y: 1, x: 2) }",
        "expected field `x` here, found `y`",
        (5, 15, "y"),
        None,
        &[],
        Some("name the fields in declaration order: `x`, `y`"),
    );
}

#[test]
fn constructor_missing_field() {
    expect_error(
        "class P {\n    x: i64\n    y: i64\n}\nfn main() { P(x: 1) }",
        "missing field `y`",
        (5, 13, "P(x: 1)"),
        None,
        &[],
        Some("name the fields in declaration order: `x`, `y`"),
    );
}

#[test]
fn constructor_extra_argument() {
    error(
        "class P {\n    x: i64\n}\nfn main() { P(x: 1, y: 2) }",
        "too many arguments: `P` has 1 field",
        (4, 21),
    );
    error(
        "class P {\n    x: i64\n    y: i64\n}\nfn main() { P(x: 1, y: 2, z: 3) }",
        "too many arguments: `P` has 2 fields",
        (5, 27),
    );
}

#[test]
fn constructor_field_type() {
    error(
        "class P {\n    x: i64\n}\nfn main() { P(x: true) }",
        "expected `i64`, found `bool`",
        (4, 18),
    );
}

#[test]
fn label_outside_constructor() {
    error(
        "fn f(a: i64) {}\nfn main() { f(a: 1) }",
        "labels on arguments come in milestone 10",
        (2, 15),
    );
}

/// A labeled argument fails the call: it is `Error`, so a later use draws nothing.
#[test]
fn labeled_call_is_error() {
    error(
        "fn f(a: i64) -> i64 { a }\nfn main() {\n    print(f(a: 1) + true)\n}",
        "labels on arguments come in milestone 10",
        (3, 13),
    );
}

/// A constructor whose own rule fails is `Error`; one with an `Error` argument keeps its
/// class, so a later use is still checked.
#[test]
fn constructor_is_error_only_when_its_rule_fails() {
    let class = "class P {\n    x: i64\n    y: i64\n}\n";
    errors(
        &format!("{class}fn main() {{\n    print(P(x: 1).x + true)\n}}"),
        &[("missing field `y`", (6, 11))],
    );
    errors(
        &format!("{class}fn main() {{\n    print(P(x: nope, y: 1).x + true)\n}}"),
        &[
            ("cannot find `nope` in this scope", (6, 16)),
            ("expected `i64`, found `bool`", (6, 32)),
        ],
    );
}

#[test]
fn constructor_reports_its_first_mistake_and_later_expression_errors() {
    let class = "class P {\n    x: i64\n    y: i64\n}\n";
    errors(
        &format!("{class}fn main() {{ P(1, nope) }}"),
        &[
            ("this argument needs a label", (5, 15)),
            ("cannot find `nope` in this scope", (5, 18)),
        ],
    );
    errors(
        &format!("{class}fn main() {{ P(1, 2) }}"),
        &[("this argument needs a label", (5, 15))],
    );
}

#[test]
fn each_label_on_a_call_is_reported() {
    errors(
        "fn f(a: i64, b: i64) {}\nfn main() { f(a: 1, b: 2) }",
        &[
            ("labels on arguments come in milestone 10", (2, 15)),
            ("labels on arguments come in milestone 10", (2, 21)),
        ],
    );
}

#[test]
fn labels_on_unknown_methods_are_reported() {
    errors(
        "class P {}\nfn main() { P().missing(x: 1 + true) }",
        &[
            ("`P` has no method `missing`", (2, 17)),
            ("labels on arguments come in milestone 10", (2, 25)),
            ("expected `i64`, found `bool`", (2, 32)),
        ],
    );
    errors(
        "fn main() { nope.m(x: 1 + true) }",
        &[
            ("cannot find `nope` in this scope", (1, 13)),
            ("labels on arguments come in milestone 10", (1, 20)),
            ("expected `i64`, found `bool`", (1, 27)),
        ],
    );
}

/// A call to an unknown name may be a misspelled constructor, whose labels are correct: only
/// the name is reported, and the arguments are still checked.
#[test]
fn misspelled_constructor_is_one_error() {
    errors(
        "class N {\n    v: i64\n}\nfn main() { Nod(v: 1) }",
        &[("cannot find function or class `Nod`", (4, 13))],
    );
    errors(
        "fn main() { nope(x: 1 + true) }",
        &[
            ("cannot find function or class `nope`", (1, 13)),
            ("expected `i64`, found `bool`", (1, 25)),
        ],
    );
}

#[test]
fn print_reports_labels_and_still_checks_its_arguments() {
    errors(
        "fn main() { print(z: 1, 2) }",
        &[
            ("`print` takes one `i64`, `u8` or `bool`", (1, 13)),
            ("labels on arguments come in milestone 10", (1, 19)),
        ],
    );
    errors(
        "class E {}\nfn main() { print(z: E()) }",
        &[
            ("labels on arguments come in milestone 10", (2, 19)),
            ("cannot print `E`", (2, 22)),
        ],
    );
}

#[test]
fn unknown_field() {
    error(
        "class P {\n    x: i64\n}\nfn main() {\n    let p = P(x: 1)\n    print(p.size)\n}",
        "`P` has no field `size`",
        (6, 13),
    );
}

#[test]
fn field_called_as_method() {
    error(
        "class P {\n    x: i64\n}\nfn main() {\n    let p = P(x: 1)\n    print(p.x())\n}",
        "`x` is a field of `P`, not a method",
        (6, 13),
    );
}

#[test]
fn method_read_as_field() {
    expect_error(
        "class P {\n    fn m(self) -> i64 { 1 }\n}\nfn main() {\n    let p = P()\n    print(p.m)\n}",
        "`m` is a method of `P`, not a field",
        (6, 13, "m"),
        None,
        &[],
        Some("call it: `m()`"),
    );
}

#[test]
fn unknown_method() {
    error(
        "class P {\n    fn m(self) -> i64 { 1 }\n}\nfn main() {\n    let p = P()\n    print(p.nope())\n}",
        "`P` has no method `nope`",
        (6, 13),
    );
}

#[test]
fn method_through_class_name() {
    error(
        "class Tree {\n    fn sum(self) -> i64 { 0 }\n}\nfn main() {\n    print(Tree.sum(Tree()))\n}",
        "cannot find `Tree` in this scope",
        (5, 11),
    );
}

#[test]
fn unknown_field_type_poisons() {
    errors(
        "class T {\n    mut x: Nope\n}\nfn main() { print(T(x: 1).x + true) }",
        &[("unknown type `Nope`", (2, 12))],
    );
}

#[test]
fn method_arity() {
    error(
        "class P {\n    fn m(self, a: i64) {}\n}\nfn main() { P().m() }",
        "`P.m` takes 1 argument, found 0",
        (4, 13),
    );
}

#[test]
fn field_on_i64() {
    error(
        "fn main() {\n    let x = 1\n    print(x.f)\n}",
        "`i64` has no field `f`",
        (3, 13),
    );
}

#[test]
fn method_on_i64() {
    error(
        "fn main() {\n    let x = 1\n    print(x.m())\n}",
        "`i64` has no method `m`",
        (3, 13),
    );
}

#[test]
fn self_outside_method() {
    error(
        "fn main() { print(self) }",
        "`self` is only available in a method",
        (1, 19),
    );
}

#[test]
fn bare_name_is_not_a_field() {
    error(
        "class P {\n    x: i64\n    fn m(self) -> i64 { x }\n}\nfn main() {}",
        "cannot find `x` in this scope",
        (3, 25),
    );
}

#[test]
fn assign_bare_field() {
    expect_error(
        "class P {\n    x: i64\n}\nfn main() {\n    let p = P(x: 1)\n    p.x = 2\n}",
        "cannot assign to `x`",
        (6, 5, "p.x = 2"),
        None,
        &[(2, 5, "declared without `mut` here")],
        Some("mark the field `mut`"),
    );
}

#[test]
fn print_class() {
    error(
        "class E {}\nfn main() { print(E()) }",
        "cannot print `E`",
        (2, 19),
    );
}

#[test]
fn poisoned_receiver_checks_args() {
    errors(
        "fn main() { nope.m(1 + true) }",
        &[
            ("cannot find `nope` in this scope", (1, 13)),
            ("expected `i64`, found `bool`", (1, 24)),
        ],
    );
}

#[test]
fn lowers_methods() {
    assert_eq!(
        lowered(
            "class C {\n    mut n: i64\n    fn bump(self, by: i64) {\n        self.n += by\n    }\n}\nfn main() {\n    let c = C(n: 1)\n    c.bump(2)\n    print(c.n)\n}"
        ),
        "(class C (mut n i64))\n\
         (fn C.bump ((self#0 C) (by#1 i64)) unit (block (block (let #2 self#0) (= (. #2 n) (+ (. #2 n) by#1)))))\n\
         (fn main () unit (block (let c#0 (new C 1)) (call C.bump c#0 2) (print (. c#0 n))))"
    );
}

#[test]
fn lowers_field_assignments() {
    assert_eq!(
        lowered(
            "class B {\n    mut f: i64\n}\nclass A {\n    mut b: B\n}\nfn mk(a: A) -> A { a }\nfn main() {\n    let a = A(b: B(f: 1))\n    a.b.f = 2\n    mk(a).b = B(f: 3)\n}"
        ),
        "(class B (mut f i64))\n\
         (class A (mut b B))\n\
         (fn mk ((a#0 A)) A (block a#0))\n\
         (fn main () unit (block (let a#0 (new A (new B 1))) (= (. (. a#0 b) f) 2) (= (. (call mk a#0) b) (new B 3))))"
    );
}

#[test]
fn assign_field_type() {
    error(
        "class P {\n    mut x: i64\n}\nfn main() {\n    let p = P(x: 1)\n    p.x = true\n}",
        "expected `i64`, found `bool`",
        (6, 11),
    );
}

#[test]
fn never_as_print_argument() {
    never_value(
        "fn f(c: bool) -> i64 {\n    print(if c { return 1 } else { return 2 })\n    1\n}\nfn main() {}",
        (2, 11),
    );
}

/// The printed `g` function, once `g`'s source sits between the spill tests' shared classes
/// and functions and an empty `main`.
fn printed_g(g: &str) -> String {
    let src = format!(
        "class B {{}}\nclass P {{\n    mut b: B\n    n: i64\n}}\nfn make() -> B {{ B() }}\nfn f(b: B, n: i64) {{}}\n{g}\nfn main() {{}}"
    );
    lowered(&src)
        .lines()
        .find(|line| line.starts_with("(fn g "))
        .expect("`g` is lowered")
        .to_owned()
}

#[test]
fn spills_call_argument() {
    assert_eq!(
        printed_g("fn g(c: bool) -> i64 {\n    f(make(), if c { return 0 } else { 1 })\n    2\n}"),
        "(fn g ((c#0 bool)) i64 (block (block (let #1 (call make)) (let #2 (if c#0 (block (return 0)) (block 1))) (call f #1 #2)) 2))"
    );
}

#[test]
fn spills_keep_source_order() {
    assert_eq!(
        printed_g(
            "fn one() -> i64 { 1 }\nfn h(a: i64, b: B, n: i64) {}\nfn g(c: bool) -> i64 {\n    h(one(), make(), if c { return 0 } else { 1 })\n    2\n}"
        ),
        "(fn g ((c#0 bool)) i64 (block (block (let #1 (call one)) (let #2 (call make)) (let #3 (if c#0 (block (return 0)) (block 1))) (call h #1 #2 #3)) 2))"
    );
}

#[test]
fn spills_constructor_argument() {
    assert_eq!(
        printed_g(
            "fn g(c: bool) -> P {\n    P(b: make(), n: if c { return P(b: B(), n: 0) } else { 1 })\n}"
        ),
        "(fn g ((c#0 bool)) P (block (block (let #1 (call make)) (let #2 (if c#0 (block (return (new P (new B) 0))) (block 1))) (new P #1 #2))))"
    );
}

#[test]
fn spills_field_assignment_base() {
    assert_eq!(
        printed_g(
            "fn g(c: bool, p: P) {\n    while true {\n        p.b = if c { break } else { make() }\n    }\n}"
        ),
        "(fn g ((c#0 bool) (p#1 P)) unit (block (loop (block (if true (block (block (let #2 p#1) (let #3 (if c#0 (block (break)) (block (call make)))) (= (. #2 b) #3))) (block (break)))))))"
    );
}

#[test]
fn no_spill_without_exit() {
    assert_eq!(
        printed_g("fn g() {\n    f(make(), 1)\n}"),
        "(fn g () unit (block (call f (call make) 1)))"
    );
}

#[test]
fn no_spill_without_earlier_reference() {
    assert_eq!(
        printed_g("fn g(c: bool) -> i64 {\n    f(if c { return 0 } else { make() }, 1)\n    2\n}"),
        "(fn g ((c#0 bool)) i64 (block (call f (if c#0 (block (return 0)) (block (call make))) 1) 2))"
    );
}

#[test]
fn nested_loop_break_still_spills() {
    assert_eq!(
        printed_g("fn g() {\n    f(make(), if true { while true { break }\n 1 } else { 1 })\n}"),
        "(fn g () unit (block (block (let #0 (call make)) (let #1 (if true (block (loop (block (if true (block (break)) (block (break))))) 1) (block 1))) (call f #0 #1))))"
    );
}

#[test]
fn spills_method_receiver() {
    assert_eq!(
        printed_g(
            "class Q {\n    fn m(self, n: i64) {}\n}\nfn q() -> Q { Q() }\nfn g(c: bool) -> i64 {\n    q().m(if c { return 0 } else { 1 })\n    2\n}"
        ),
        "(fn g ((c#0 bool)) i64 (block (block (let #1 (call q)) (let #2 (if c#0 (block (return 0)) (block 1))) (call Q.m #1 #2)) 2))"
    );
}

/// `a.f op= e` reads its bound `a` as the assignment's base before `e` runs.
#[test]
fn spills_compound_field_assignment_base() {
    assert_eq!(
        printed_g(
            "class Q {\n    mut k: i64\n}\nfn g(c: bool, q: Q) {\n    while true {\n        q.k += if c { break } else { 1 }\n    }\n}"
        ),
        "(fn g ((c#0 bool) (q#1 Q)) unit (block (loop (block (if true (block (block (let #2 q#1) (block (let #3 #2) (let #4 (+ (. #2 k) (if c#0 (block (break)) (block 1)))) (= (. #3 k) #4)))) (block (break)))))))"
    );
}

#[test]
fn spills_every_operand_through_the_last_exit() {
    assert_eq!(
        printed_g(
            "fn one() -> i64 { 1 }\nfn h(b: B, x: i64, y: i64, z: i64) {}\nfn g(c: bool) -> i64 {\n    h(make(), if c { return 0 } else { 1 }, if c { return 1 } else { 2 }, one())\n    2\n}"
        ),
        "(fn g ((c#0 bool)) i64 (block (block (let #1 (call make)) (let #2 (if c#0 (block (return 0)) (block 1))) (let #3 (if c#0 (block (return 1)) (block 2))) (call h #1 #2 #3 (call one))) 2))"
    );
}

#[test]
fn spills_for_an_exit_in_a_nested_assignment() {
    assert_eq!(
        printed_g(
            "fn g(c: bool) -> i64 {\n    let mut x = 0\n    f(make(), if true {\n        x = if c { return 0 } else { 1 }\n        x\n    } else { 0 })\n    2\n}"
        ),
        "(fn g ((c#0 bool)) i64 (block (let mut x#1 0) (block (let #2 (call make)) (let #3 (if true (block (= x#1 (if c#0 (block (return 0)) (block 1))) x#1) (block 0))) (call f #2 #3)) 2))"
    );
}

#[test]
fn no_spill_with_only_scalars_before_the_exit() {
    assert_eq!(
        printed_g(
            "fn one() -> i64 { 1 }\nfn h2(a: i64, n: i64) {}\nfn g(c: bool) -> i64 {\n    h2(one(), if c { return 0 } else { 1 })\n    2\n}"
        ),
        "(fn g ((c#0 bool)) i64 (block (call h2 (call one) (if c#0 (block (return 0)) (block 1))) 2))"
    );
}

/// A class-optional operand holds a reference, and an `if let` can exit.
#[test]
fn spills_class_optional_before_exit_in_if_let() {
    assert_eq!(
        printed_g(
            "fn maybe() -> B? { None }\nfn h(b: B?, n: i64) {}\nfn g(c: bool, n: B?) -> i64 {\n    h(maybe(), if let x = n { return 0 } else { 1 })\n    2\n}"
        ),
        "(fn g ((c#0 bool) (n#1 B?)) i64 (block (block (let #3 (call maybe)) (let #4 (if-some x#2 n#1 (block (return 0)) (block 1))) (call h #3 #4)) 2))"
    );
}

/// The class the optionals tests share: its `next` field is optional.
const T: &str = "class T {\n    v: i64\n    next: T?\n}\n";

/// Asserts that `f`, lowered after `prefix` and before an empty `main`, prints `part`.
fn f_contains(prefix: &str, f: &str, part: &str) {
    let printed = lowered(&format!("{prefix}{f}\nfn main() {{}}"));
    assert!(printed.contains(part), "{printed}");
}

#[test]
fn none_needs_a_type() {
    expect_error(
        "fn main() { let x = None }",
        "cannot infer the type of `None`",
        (1, 21, "None"),
        None,
        &[],
        Some("write the type: `let x: Tree? = None`"),
    );
}

#[test]
fn if_of_two_nones() {
    errors(
        "class T {}\nfn main() {\n    let x = if true { None } else { None }\n}",
        &[("cannot infer the type of `None`", (3, 23))],
    );
}

#[test]
fn none_for_non_optional() {
    error(
        "fn main() { let x: i64 = None }",
        "expected `i64`, found `None`",
        (1, 26),
    );
}

#[test]
fn dot_on_optional() {
    expect_error(
        "class T {\n    v: i64\n}\nfn f(node: T?) -> i64 { node.v }\nfn main() {}",
        "`node` may be `None`",
        (4, 25, "node"),
        None,
        &[],
        Some("use `?.`, or unwrap it with `if let`"),
    );
}

/// A path receiver is quoted as written; any other is "this value". A method call too.
#[test]
fn optional_receiver_is_named_by_its_path() {
    errors(
        "class N {\n    next: N?\n    fn m(self) -> i64 { 1 }\n}\nfn g() -> N? { None }\nfn f(n: N) {\n    print(n.next.m())\n    print(g().next)\n}\nfn main() {}",
        &[
            ("`n.next` may be `None`", (7, 11)),
            ("this value may be `None`", (8, 11)),
        ],
    );
}

#[test]
fn optional_self_field_is_named_by_its_path() {
    expect_error(
        "class N {\n    next: N?\n    v: i64\n    fn m(self) -> i64 { self.next.v }\n}\nfn main() {}",
        "`self.next` may be `None`",
        (4, 25, "self.next"),
        None,
        &[],
        Some("use `?.`, or unwrap it with `if let`"),
    );
}

#[test]
fn if_let_needs_optional() {
    error(
        "fn main() {\n    if let x = 5 {}\n}",
        "`if let` needs an optional; this is `i64`",
        (2, 16),
    );
}

#[test]
fn while_let_needs_optional() {
    error(
        "fn main() {\n    while let x = 5 {}\n}",
        "`while let` needs an optional; this is `i64`",
        (2, 19),
    );
}

#[test]
fn failed_if_let_binding_poisons() {
    errors(
        "fn main() {\n    if let x = 5 {\n        print(x + true)\n    }\n}",
        &[("`if let` needs an optional; this is `i64`", (2, 16))],
    );
}

#[test]
fn if_let_scope() {
    error(
        &format!(
            "{T}fn f(n: T?) {{\n    if let x = n {{}} else {{ let y = x }}\n}}\nfn main() {{}}"
        ),
        "cannot find `x` in this scope",
        (6, 36),
    );
}

#[test]
fn optional_breaks_the_cycle() {
    clean(&format!("{T}fn main() {{}}"));
}

#[test]
fn none_from_every_source() {
    clean(&format!(
        "{T}fn take(t: T?) {{}}\nfn give() -> T? {{ None }}\nfn main() {{\n    let a: T? = None\n    let b = T(v: 1, next: None)\n    take(None)\n    let mut c: T? = if true {{ None }} else if false {{ None }} else {{ None }}\n    c = None\n    return\n}}\nfn back(c: bool) -> T? {{\n    if c {{ return None }}\n    if let x = give() {{ None }} else {{ None }}\n}}"
    ));
}

#[test]
fn assign_if_let_binding() {
    expect_error(
        &format!(
            "{T}fn f(n: T?) {{\n    if let x = n {{\n        x = T(v: 1, next: None)\n    }}\n}}\nfn main() {{}}"
        ),
        "cannot assign to `x`",
        (7, 9, "x = T(v: 1, next: None)"),
        None,
        &[(6, 12, "bound by `if let` here")],
        None,
    );
}

#[test]
fn assign_while_let_binding() {
    expect_error(
        &format!(
            "{T}fn f(n: T?) {{\n    while let x = n {{\n        x = T(v: 1, next: None)\n    }}\n}}\nfn main() {{}}"
        ),
        "cannot assign to `x`",
        (7, 9, "x = T(v: 1, next: None)"),
        None,
        &[(6, 15, "bound by `while let` here")],
        None,
    );
}

#[test]
fn print_optional() {
    error(
        &format!("{T}fn f(x: T?) {{ print(x) }}\nfn main() {{}}"),
        "`print` takes one `i64`, `u8` or `bool`",
        (5, 15),
    );
    error(
        "fn f(x: i64?) { print(x) }\nfn main() {}",
        "`print` takes one `i64`, `u8` or `bool`",
        (1, 17),
    );
}

/// A `None` that its rule rejects fails that rule: what the rule makes is `Error`, so a
/// later use draws nothing.
#[test]
fn rejected_none_reports_once() {
    let cases = [
        (
            "fn main() {\n    let x = !None\n    print(x + 1)\n}",
            "expected `bool`, found `None`",
            (2, 14),
        ),
        (
            "fn main() {\n    let x = if None { 1 } else { 2 }\n    print(x && true)\n}",
            "expected `bool`, found `None`",
            (2, 16),
        ),
        (
            "fn f(x: i64) -> i64 { x }\nfn main() {\n    let r = f(None)\n    print(r && true)\n}",
            "expected `i64`, found `None`",
            (3, 15),
        ),
        (
            "fn main() {\n    let y = None + 1\n    print(y && true)\n}",
            "expected `i64`, found `None`",
            (2, 13),
        ),
    ];
    for (src, message, pos) in cases {
        error(src, message, pos);
    }
    // Each `None` is its own mistake; the `if` that holds them fails the rule too.
    let nested = [
        (
            "fn f(c: bool) {\n    let x = !(if c { None } else { None })\n    print(x + 1)\n}"
                .to_owned(),
            [22, 36].as_slice(),
            2,
        ),
        (
            "fn f(c: bool) {\n    let x = !(if c { if c { None } else { None } } else { None })\n    print(x + 1)\n}"
                .to_owned(),
            &[29, 43, 59],
            2,
        ),
        (
            format!("{T}fn f(n: T?) {{\n    let x = !(if let m = n {{ None }} else {{ None }})\n    print(x + 1)\n}}"),
            &[30, 44],
            6,
        ),
    ];
    for (src, cols, line) in nested {
        let expected: Vec<_> = cols
            .iter()
            .map(|&col| ("expected `bool`, found `None`", (line, col)))
            .collect();
        errors(&format!("{src}\nfn main() {{}}"), &expected);
    }
    // A rejected condition or scrutinee keeps the branches' mismatch.
    errors(
        "fn main() {\n    let x = !(if 1 { None } else { None })\n    print(x + 1)\n}",
        &[
            ("expected `bool`, found `i64`", (2, 18)),
            ("expected `bool`, found `None`", (2, 22)),
            ("expected `bool`, found `None`", (2, 36)),
        ],
    );
    errors(
        "fn main() {\n    let x = !(if let y = 5 { None } else { None })\n    print(x + 1)\n}",
        &[
            ("`if let` needs an optional; this is `i64`", (2, 26)),
            ("expected `bool`, found `None`", (2, 30)),
            ("expected `bool`, found `None`", (2, 44)),
        ],
    );
}

/// An `if let` whose value is not optional fails: it is `Error`, so a later use draws
/// nothing.
#[test]
fn rejected_if_let_scrutinee_reports_once() {
    error(
        "fn main() {\n    let x = if let y = 5 { 1 } else { 2 }\n    print(x && true)\n}",
        "`if let` needs an optional; this is `i64`",
        (2, 24),
    );
}

/// Branches that are `Error` by their own mistakes did not reject the type the `!` passed
/// down, so the `!` is still `bool`, and a later use is checked against it.
#[test]
fn error_branches_without_mismatch_keep_the_rule_type() {
    errors(
        "fn f(c: bool) {\n    let x = !(if c { nope } else { nope })\n    print(x + 1)\n}\nfn main() {}",
        &[
            ("cannot find `nope` in this scope", (2, 22)),
            ("cannot find `nope` in this scope", (2, 36)),
            ("expected `i64`, found `bool`", (3, 11)),
        ],
    );
}

/// A body whose tail failed keeps its type, so a `T?` function still wraps its `T` value
/// and nothing more is reported.
#[test]
fn failed_tail_of_an_optional_body_reports_once() {
    error(
        "fn f(c: bool) -> i64? {\n    let a = 1\n    if c { nope } else { a }\n}\nfn main() {}",
        "cannot find `nope` in this scope",
        (3, 12),
    );
}

/// With a type expected, a join that fails is reported at the branch that fits it neither
/// as is nor as its payload, in either order, from a body, a `return` or a `let`.
#[test]
fn failed_join_blames_the_branch_that_does_not_fit() {
    let sites = [
        ("fn f(c: bool) -> T? {\n    ", 5),
        ("fn f(c: bool) -> T? {\n    return ", 12),
        ("fn f(c: bool) {\n    let x: T? = ", 17),
    ];
    let values = [
        ("if c { 1 } else { None }", 7),
        ("if c { None } else { 1 }", 21),
    ];
    for (site, start) in sites {
        for (value, offset) in values {
            error(
                &format!("{T}{site}{value}\n}}\nfn main() {{}}"),
                "expected `T?`, found `i64`",
                (6, start + offset),
            );
        }
    }
}

/// A `unit` or `never` branch types no `None` beside it.
#[test]
fn none_beside_unit_or_never_needs_a_type() {
    for (value, col) in [
        ("if c { g() } else { None }", 25),
        ("if c { return } else { None }", 28),
    ] {
        errors(
            &format!(
                "fn g() {{}}\nfn f(c: bool) {{\n    {value}\n    print(1)\n}}\nfn main() {{}}"
            ),
            &[("cannot infer the type of `None`", (3, col))],
        );
    }
}

/// The `if let` and `while let` binding belongs to the scope of its block, so the block
/// cannot declare the name again.
#[test]
fn unwrapping_block_cannot_redeclare_the_binding() {
    for (head, x, col) in [("if let", 12, 24), ("while let", 15, 27)] {
        expect_error(
            &format!("{T}fn f(n: T?) {{\n    {head} x = n {{ let x = 1 }}\n}}\nfn main() {{}}"),
            "`x` is already declared in this scope",
            (6, col, "x"),
            None,
            &[(6, x, "first declared here")],
            None,
        );
    }
}

/// Every position that expects a `T?` wraps a `T`.
#[test]
fn wraps_where_optional_expected() {
    let src = "fn g(x: T?) {}\nfn f(t: T) -> T? {\n    let a: T? = t\n    let mut b: T? = None\n    b = t\n    g(t)\n    let c = T(v: 1, next: t)\n    return t\n}";
    for part in [
        "(let a#1 (wrap t#0))",
        "(= b#2 (wrap t#0))",
        "(call g (wrap t#0))",
        "(new T 1 (wrap t#0))",
        "(return (wrap t#0))",
    ] {
        f_contains(T, src, part);
    }
}

#[test]
fn lowers_else_if_let() {
    f_contains(
        T,
        "fn f(a: T?, b: T?) -> i64 {\n    if let x = a { x.v } else if let y = b { y.v } else { 0 }\n}",
        "(if-some x#2 a#0 (block (. x#2 v)) (block (if-some y#3 b#1 (block (. y#3 v)) (block 0))))",
    );
}

#[test]
fn if_let_shadows() {
    clean(&format!(
        "{T}fn f(node: T?) -> i64 {{\n    if let node = node {{ node.v }} else {{ 0 }}\n}}\nfn main() {{}}"
    ));
}

/// The `?` is not part of the unknown name, and the `None` it would type draws nothing.
#[test]
fn unknown_optional_type_underlines_the_name() {
    let src = "fn main() { let x: Foo? = None }";
    let ds = diags(src);
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].message, "unknown type `Foo`");
    assert_eq!(&src[ds[0].span.start..ds[0].span.end], "Foo");
}

#[test]
fn wraps_a_value_where_optional_expected() {
    f_contains(
        T,
        "fn f() -> T? { T(v: 1, next: None) }",
        "(block (wrap (new T 1 None)))",
    );
}

#[test]
fn joins_if_branches() {
    f_contains(
        T,
        "fn f(c: bool, t: T) -> T? {\n    let x = if c { None } else { t }\n    x\n}",
        "(let x#2 (if c#0 (block None) (block (wrap t#1))))",
    );
}

/// The spec's `if c { t } else { None }`: the value comes first, and its branch is wrapped.
#[test]
fn joins_a_value_then_none() {
    f_contains(
        T,
        "fn f(c: bool, t: T) -> T? {\n    let x = if c { t } else { None }\n    x\n}",
        "(let x#2 (if c#0 (block (wrap t#1)) (block None)))",
    );
}

#[test]
fn wraps_an_if_let_binding_beside_none() {
    f_contains(
        T,
        "fn f(n: T?) {\n    let y = if let x = n { x } else { None }\n}",
        "(let y#2 (if-some x#1 n#0 (block (wrap x#1)) (block None)))",
    );
}

/// A `T` meets a `T?` that is not a bare `None`: the `T` branch is wrapped.
#[test]
fn wraps_a_value_beside_an_optional_call() {
    f_contains(
        T,
        "fn give() -> T? { None }\nfn f(c: bool, t: T) {\n    let x = if c { t } else { give() }\n}",
        "(let x#2 (if c#0 (block (wrap t#1)) (block (call give))))",
    );
}

#[test]
fn lowers_if_let() {
    f_contains(
        T,
        "fn f(n: T?) -> i64 {\n    if let x = n { x.v } else { 0 }\n}",
        "(if-some x#1 n#0 (block (. x#1 v)) (block 0))",
    );
}

#[test]
fn lowers_while_let() {
    f_contains(
        T,
        "fn f(l: T?) -> i64 {\n    let mut n = 0\n    let mut node = l\n    while let x = node {\n        n += 1\n        node = x.next\n    }\n    n\n}",
        "(loop (block (if-some x#3 node#2 (block (= n#1 (+ n#1 1)) (= node#2 (. x#3 next))) (block (break)))))",
    );
}

#[test]
fn safe_call_on_non_optional() {
    expect_error(
        "class T {\n    v: i64\n}\nfn f(t: T) -> i64? { t?.v }\nfn main() {}",
        "`t` is not optional",
        (4, 22, "t"),
        None,
        &[],
        Some("use `.`"),
    );
}

/// In `a?.b.c`, the `.c` applies to a `B?`.
#[test]
fn safe_chain_needs_each_step() {
    expect_error(
        "class A {\n    b: B\n}\nclass B {\n    c: i64\n}\nfn f(a: A?) -> i64? { a?.b.c }\nfn main() {}",
        "`a?.b` may be `None`",
        (7, 23, "a?.b"),
        None,
        &[],
        Some("use `?.`, or unwrap it with `if let`"),
    );
}

#[test]
fn coalesce_left_not_optional() {
    error(
        "fn main() { print(1 ?? 2) }",
        "the left side of `??` is not optional",
        (1, 19),
    );
}

#[test]
fn coalesce_never() {
    never_value(
        "fn f(x: i64?, c: bool) -> i64 {\n    x ?? if c { return 0 } else { return 1 }\n}\nfn main() {}",
        (2, 10),
    );
}

#[test]
fn coalesce_type() {
    error(
        "fn f(x: i64?) -> i64 { x ?? true }\nfn main() {}",
        "expected `i64`, found `bool`",
        (1, 29),
    );
}

#[test]
fn equal_none_on_non_optional() {
    error(
        "class T {}\nfn f(x: T) -> bool { x == None }\nfn main() {}",
        "`x` is not optional, so it is never `None`",
        (2, 22),
    );
}

#[test]
fn none_equals_none() {
    errors(
        "fn main() { print(None == None) }",
        &[("cannot infer the type of `None`", (1, 19))],
    );
}

#[test]
fn equal_unit() {
    errors(
        "fn g() {}\nfn main() { print(g() == g()) }",
        &[
            ("expression has no value", (2, 19)),
            ("expression has no value", (2, 26)),
        ],
    );
}

#[test]
fn equal_two_classes() {
    error(
        "class A {}\nclass B {}\nfn f(a: A, b: B) -> bool { a == b }\nfn main() {}",
        "cannot compare `A` with `B`",
        (3, 28),
    );
}

#[test]
fn is_none() {
    expect_error(
        "class T {}\nfn f(x: T?) -> bool { x is None }\nfn main() {}",
        "use `==` to test for `None`",
        (2, 23, "x is None"),
        None,
        &[],
        Some("`x == None`"),
    );
}

#[test]
fn is_on_integers() {
    expect_error(
        "fn main() { print(1 is 1) }",
        "`is` needs objects; found `i64`",
        (1, 19, "1"),
        None,
        &[],
        Some("use `==` to compare values"),
    );
}

/// The operand that is not an object is blamed, not the whole expression.
#[test]
fn is_blames_the_operand_that_is_not_an_object() {
    expect_error(
        "class T {}\nfn f(t: T) -> bool { t is 1 }\nfn main() {}",
        "`is` needs objects; found `i64`",
        (2, 27, "1"),
        None,
        &[],
        Some("use `==` to compare values"),
    );
}

/// A side with no path has no help, as `this value == None` is not code.
#[test]
fn is_none_without_a_path_has_no_help() {
    expect_error(
        "class T {}\nfn make() -> T? { None }\nfn f() -> bool { make() is None }\nfn main() {}",
        "use `==` to test for `None`",
        (3, 18, "make() is None"),
        None,
        &[],
        None,
    );
}

/// `t == None` does not compile for a `T`, so it is not offered.
#[test]
fn is_none_on_a_plain_object_has_no_help() {
    expect_error(
        "class T {}\nfn f(t: T) -> bool { t is None }\nfn main() {}",
        "use `==` to test for `None`",
        (2, 22, "t is None"),
        None,
        &[],
        None,
    );
}

#[test]
fn is_none_checks_the_other_side() {
    errors(
        "fn main() { print(nope is None) }",
        &[
            ("use `==` to test for `None`", (1, 19)),
            ("cannot find `nope` in this scope", (1, 19)),
        ],
    );
}

/// Once an operand is `Error`, the later one is not held to `is`.
#[test]
fn is_after_an_error_checks_no_further() {
    errors(
        "fn main() { print(nope is 1) }",
        &[("cannot find `nope` in this scope", (1, 19))],
    );
}

#[test]
fn is_two_classes() {
    error(
        "class A {}\nclass B {}\nfn f(a: A, b: B) -> bool { a is b }\nfn main() {}",
        "cannot compare `A` with `B`",
        (3, 28),
    );
}

/// The other operand is still checked after a `??` or `?.` fails.
#[test]
fn coalesce_left_not_optional_checks_the_right() {
    errors(
        "fn main() { print(1 ?? nope) }",
        &[
            ("the left side of `??` is not optional", (1, 19)),
            ("cannot find `nope` in this scope", (1, 24)),
        ],
    );
}

#[test]
fn safe_call_on_non_optional_checks_the_member() {
    errors(
        "class T {\n    v: i64\n}\nfn f(t: T) -> i64? { t?.w }\nfn main() {}",
        &[
            ("`t` is not optional", (4, 22)),
            ("`T` has no field `w`", (4, 25)),
        ],
    );
}

/// The member's argument is checked on the plain receiver too.
#[test]
fn safe_call_on_non_optional_checks_the_arguments() {
    errors(
        "class T {\n    fn g(self, x: i64) -> i64 { x }\n}\nfn f(t: T) -> i64? { t?.g(nope) }\nfn main() {}",
        &[
            ("`t` is not optional", (4, 22)),
            ("cannot find `nope` in this scope", (4, 27)),
        ],
    );
}

/// The class the `?.`, `??`, `==` and `is` lowering tests share.
const C: &str =
    "class T {\n    v: i64\n    o: T?\n    fn m(self) {}\n    fn g(self, x: i64) -> i64 { x }\n}\n";

#[test]
fn lowers_safe_field() {
    f_contains(
        C,
        "fn f(t: T?) -> i64? { t?.v }",
        "(if-some #1 t#0 (block (wrap (. #1 v))) (block None))",
    );
}

#[test]
fn safe_optional_field_not_wrapped() {
    f_contains(
        C,
        "fn f(t: T?) -> T? { t?.o }",
        "(if-some #1 t#0 (block (. #1 o)) (block None))",
    );
}

#[test]
fn lowers_safe_unit_method() {
    f_contains(
        C,
        "fn f(t: T?) { t?.m() }",
        "(if-some #1 t#0 (block (call T.m #1)) (block))",
    );
}

#[test]
fn lowers_safe_method_with_an_argument() {
    f_contains(
        C,
        "fn f(t: T?) -> i64? { t?.g(1) }",
        "(if-some #1 t#0 (block (wrap (call T.g #1 1))) (block None))",
    );
}

#[test]
fn lowers_coalesce() {
    f_contains(
        C,
        "fn f(x: i64?) -> i64 { x ?? 0 }",
        "(if-some #1 x#0 (block #1) (block 0))",
    );
}

#[test]
fn coalesce_with_optional_right() {
    f_contains(
        C,
        "fn f(x: i64?, y: i64?) -> i64? { x ?? y }",
        "(if-some #2 x#0 (block (wrap #2)) (block y#1))",
    );
}

/// A bare `None` on the right takes the left side's type.
#[test]
fn coalesce_with_none_right() {
    f_contains(
        C,
        "fn f(x: i64?) -> i64? { x ?? None }",
        "(if-some #1 x#0 (block (wrap #1)) (block None))",
    );
}

#[test]
fn coalesce_with_none_in_an_if() {
    clean(&format!(
        "{C}fn f(x: i64?, c: bool) -> i64? {{ x ?? if c {{ None }} else {{ 1 }} }}\nfn main() {{}}"
    ));
}

/// `x ?? (y ?? 0)` is an `i64`, which the return type checks.
#[test]
fn coalesce_chain_is_not_optional() {
    clean(&format!(
        "{C}fn f(x: i64?, y: i64?) -> i64 {{ x ?? y ?? 0 }}\nfn main() {{}}"
    ));
}

#[test]
fn equal_wraps_the_plain_side() {
    f_contains(
        C,
        "fn f(a: i64?, b: i64) -> bool { a == b }",
        "(== a#0 (wrap b#1))",
    );
}

/// A bare `None` on the left takes the type of the right side.
#[test]
fn none_on_the_left_takes_the_other_type() {
    f_contains(C, "fn f(t: T?) -> bool { None != t }", "(!= None t#0)");
}

#[test]
fn is_wraps_the_plain_side() {
    f_contains(
        C,
        "fn f(a: T, b: T?) -> bool { a is b }",
        "(is (wrap a#0) b#1)",
    );
}

#[test]
fn spills_is() {
    f_contains(
        C,
        "fn make() -> T { T(v: 0, o: None) }\nfn f(c: bool) -> bool {\n    make() is if c { return false } else { make() }\n}",
        "(block (let #1 (call make)) (let #2 (if c#0 (block (return false)) (block (call make)))) (is #1 #2))",
    );
}

#[test]
fn spills_equal() {
    f_contains(
        C,
        "fn make() -> T { T(v: 0, o: None) }\nfn f(c: bool) -> bool {\n    make() == if c { return false } else { make() }\n}",
        "(block (let #1 (call make)) (let #2 (if c#0 (block (return false)) (block (call make)))) (== #1 #2))",
    );
}

/// `print` of a value it cannot print names the argument, not the call; a bad argument count
/// stays on the whole call.
#[test]
fn print_argument_errors_sit_on_the_argument() {
    errors(
        "class P {\n    x: i64\n}\nfn nothing() {}\nfn main() {\n    print(P(x: 1))\n    print(nothing())\n    print([1])\n}",
        &[
            ("cannot print `P`", (6, 11)),
            ("expression has no value", (7, 11)),
            ("cannot print `[i64]`", (8, 11)),
        ],
    );
}

/// Two members of the same kind are "defined twice"; a method and a field of one name name the
/// first one's kind.
#[test]
fn duplicate_member_of_the_same_kind() {
    expect_error(
        "class A {\n    fn x(self) {}\n    fn x(self) {}\n}\nfn main() {}",
        "method `x` is defined twice",
        (3, 8, "x"),
        None,
        &[(2, 8, "first defined here")],
        None,
    );
    expect_error(
        "class A {\n    x: i64\n    x: i64\n}\nfn main() {}",
        "field `x` is defined twice",
        (3, 5, "x"),
        None,
        &[(2, 5, "first defined here")],
        None,
    );
}

#[test]
fn unit_branch_says_no_value() {
    expect_error(
        "fn nothing() {}\nfn main() { let y: i64 = if true { 1 } else { nothing() } }",
        "expression has no value",
        (2, 47, "nothing()"),
        None,
        &[],
        None,
    );
    expect_error(
        "fn nothing() {}\nfn main() { let y: i64 = if true { nothing() } else { 1 } }",
        "expression has no value",
        (2, 36, "nothing()"),
        None,
        &[],
        None,
    );
    // Without a type expected, the branch with no value is still the one named.
    expect_error(
        "fn nothing() {}\nfn main() { let x = if true { nothing() } else { 1 } }",
        "expression has no value",
        (2, 31, "nothing()"),
        None,
        &[],
        None,
    );
}

/// Branches of two other types still report a mismatch, and never say `unit`.
#[test]
fn value_branches_of_different_types_still_mismatch() {
    error(
        "fn main() { let x = if true { 1 } else { false } }",
        "expected `i64`, found `bool`",
        (1, 42),
    );
}

#[test]
fn if_with_two_never_branches_has_no_value() {
    never_value(
        "fn f(c: bool) -> i64 {\n    let z = if c { return 1 } else { return 2 }\n    z\n}\nfn main() {}",
        (2, 13),
    );
}

#[test]
fn assigning_to_a_function_names_it() {
    for (src, col) in [
        ("fn one() {}\nfn main() { one = 2 }", 13),
        ("fn one() {}\nfn main() { one += 2 }", 13),
        ("fn one() {}\nfn main() { one[0] = 2 }", 13),
    ] {
        expect_error(
            src,
            "`one` is a function, not a variable",
            (2, col, "one"),
            None,
            &[],
            None,
        );
    }
}

/// A local that shadows a function is assigned, not reported.
#[test]
fn local_shadowing_a_function_is_assignable() {
    clean("fn one() {}\nfn main() {\n    let mut one = 1\n    one = 2\n    one += 1\n}");
}
