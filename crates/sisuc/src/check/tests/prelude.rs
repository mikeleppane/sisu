//! The prelude: `print` and `panic`, the names no top-level item can take.

use super::*;

/// A function `f` over `params`, whose body is `body` from line 2, and an empty `main`.
fn body(params: &str, body: &str) -> String {
    format!("fn f({params}) {{\n    {body}\n}}\nfn main() {{}}")
}

#[test]
fn panic_lowers() {
    let (program, ds) = checked("fn main() {\n    panic(\"x\")\n}");
    assert!(ds.is_empty(), "{ds:?}");
    let program = program.expect("no errors");
    assert!(program.to_string().contains("(panic \"x\")"), "{program}");
    let panic = program.functions[0]
        .body
        .value
        .as_ref()
        .expect("`panic(\"x\")` should be the body's value");
    assert_eq!(panic.ty, Type::Never);
}

/// A message that is not a `string`: in each row, the body, the argument reported, and the
/// body the certain fix makes, if there is one.
#[test]
fn panic_message_must_be_string() {
    for (statements, pos, fixed) in [
        ("panic(1)", (2, 11, "1"), Some("panic(\"{1}\")")),
        ("panic(true)", (2, 11, "true"), Some("panic(\"{true}\")")),
        (
            "let c = 'a'\n    panic(c)",
            (3, 11, "c"),
            Some("let c = 'a'\n    panic(\"{c}\")"),
        ),
        ("let xs = [1]\n    panic(xs)", (3, 11, "xs"), None),
        ("panic(1 +\n 2)", (2, 11, "1 +\n 2"), None),
    ] {
        let src = body("", statements);
        let d = diagnostic(&src, Code::Mismatch, pos, &[]);
        match fixed {
            Some(fixed) => {
                assert!(fixes(&src, &d).iter().all(|(certain, _)| *certain), "{src}");
                assert_eq!(applied(&src, &d), body("", fixed));
                fix_removes_its_code(&src, &d);
            }
            None => assert_eq!(fixes(&src, &d), [], "{src}"),
        }
    }
}

#[test]
fn panic_argument_count() {
    for statement in ["panic()", "panic(\"a\", \"b\")"] {
        codes(&body("", statement), &[(Code::ArgumentCount, (2, 5))]);
    }
}

/// `panic` never finishes: a branch, a body's tail and the right side of `??` take it.
#[test]
fn panic_is_never() {
    for src in [
        body("", "let n: i64? = None\n    let m = n ?? panic(\"x\")"),
        body("c: bool", "let n = if c { 1 } else { panic(\"x\") }"),
        "fn f() -> i64 {\n    panic(\"x\")\n}\nfn main() {}".to_string(),
    ] {
        clean(&src);
    }
    codes(
        &body("", "panic(\"x\")\n    print(1)"),
        &[(Code::UnreachableCode, (3, 5))],
    );
}

/// Anywhere else a value is used, `panic` is the "no value" error, as a `return` is.
#[test]
fn panic_is_not_a_value() {
    for (statement, col) in [
        ("let x = panic(\"x\")", 13),
        ("g(panic(\"x\"))", 7),
        ("let y = 1 + panic(\"x\")", 17),
    ] {
        let src = format!("fn g(n: i64) {{}}\n{}", body("", statement));
        codes(&src, &[(Code::NoValue, (3, col))]);
    }
}

/// A `unit` right side of `??` is the "no value" error, reported once.
#[test]
fn coalesce_unit_is_not_a_value() {
    codes(
        "fn g() {}\nfn f(x: i64?) {\n    print(x ?? g())\n}\nfn main() {}",
        &[(Code::NoValue, (3, 16))],
    );
}

/// No top-level function, class or constant takes a prelude name.
#[test]
fn prelude_defined_twice() {
    for name in ["panic", "print"] {
        for (item, col) in [
            (format!("fn {name}() {{}}"), 4),
            (format!("class {name} {{}}"), 7),
            (format!("const {name} = 1"), 7),
        ] {
            let src = format!("{item}\nfn main() {{}}");
            diagnostic(&src, Code::DefinedTwice, (1, col, name), &[]);
        }
    }
}

/// A local, a parameter and a method may take a prelude name.
#[test]
fn prelude_names_free_elsewhere() {
    clean(
        "class P {\n    fn print(self) {}\n}\nfn f(panic: i64) {\n    let print = 1\n}\nfn main() {}",
    );
}
