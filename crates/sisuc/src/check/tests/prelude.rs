//! The prelude: `print` and `panic`, the names no top-level item can take, and `todo`.

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
            "let b = u8(1)\n    panic(b)",
            (3, 11, "b"),
            Some("let b = u8(1)\n    panic(\"{b}\")"),
        ),
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

/// A message that fails to check makes the `panic` call `Error`, as a failed argument rule
/// does a call: only the message's own error is reported, not "no value" for the call too.
#[test]
fn panic_with_a_broken_message_is_an_error() {
    for (statement, expected) in [
        (
            "let x = panic(missing)",
            &[(Code::BindingNotFound, (3, 19))][..],
        ),
        (
            "let x = panic(\"{missing}\")",
            &[(Code::BindingNotFound, (3, 21))],
        ),
        ("let x = panic(panic(\"x\"))", &[(Code::NoValue, (3, 19))]),
        ("let x = panic(1)", &[(Code::Mismatch, (3, 19))]),
        ("g(panic(1))", &[(Code::Mismatch, (3, 13))]),
        ("let y = 1 + panic(1)", &[(Code::Mismatch, (3, 23))]),
        (
            "let n: i64? = None\n    let m = n ?? panic(1)",
            &[(Code::Mismatch, (4, 24))],
        ),
    ] {
        let src = format!("fn g(n: i64) {{}}\n{}", body("", statement));
        codes(&src, expected);
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

#[test]
fn todo_lowers() {
    let src =
        "fn f() -> i64 {\n    todo\n}\nfn g() -> string {\n    todo(\"x {1}\")\n}\nfn main() {}";
    codes(src, &[(Code::TodoLeft, (2, 5)), (Code::TodoLeft, (5, 5))]);
    let printed = checked(src).0.expect("only warnings").to_string();
    for part in [
        "(panic \"not yet implemented\")",
        "(panic (concat \"not yet implemented: \" ",
    ] {
        assert!(printed.contains(part), "{part} in {printed}");
    }
}

/// `todo`'s message follows `panic`'s rules: in each row, the body, the other diagnostic beside
/// the warning, and whether its fix wraps the message.
#[test]
fn todo_message_rules() {
    for (statements, (code, pos), wraps) in [
        ("todo(1)", (Code::Mismatch, (2, 10)), true),
        ("todo(\"a\", \"b\")", (Code::ArgumentCount, (2, 5)), false),
        (
            "let xs = [1]\n    todo(xs)",
            (Code::Mismatch, (3, 10)),
            false,
        ),
    ] {
        let src = body("", statements);
        codes(&src, &[(Code::TodoLeft, (pos.0, 5)), (code, pos)]);
        let d = &diags(&src)[1];
        if wraps {
            assert_eq!(applied(&src, d), body("", "todo(\"{1}\")"));
            fix_removes_its_code(&src, d);
        } else {
            assert_eq!(fixes(&src, d), [], "{src}");
        }
    }
}

/// `todo` never finishes, as `panic` does: only the warning is reported.
#[test]
fn todo_is_never() {
    for (src, expected) in [
        (
            body("", "let n: i64? = None\n    print(n ?? todo)"),
            &[(Code::TodoLeft, (3, 16))][..],
        ),
        (
            body("c: bool", "let n = if c { 1 } else { todo }"),
            &[(Code::TodoLeft, (2, 31))],
        ),
        (
            body("", "todo\n    print(1)"),
            &[(Code::TodoLeft, (2, 5)), (Code::UnreachableCode, (3, 5))],
        ),
    ] {
        codes(&src, expected);
    }
}

/// Anywhere else a value is used, `todo` is the "no value" error, as `panic` is.
#[test]
fn todo_is_not_a_value() {
    for (statement, col) in [
        ("let x = todo", 13),
        ("g(todo)", 7),
        ("let y = 1 + todo", 17),
    ] {
        let src = format!("fn g(n: i64) {{}}\n{}", body("", statement));
        codes(
            &src,
            &[(Code::TodoLeft, (3, col)), (Code::NoValue, (3, col))],
        );
    }
}

/// A labelled message is reported once, and makes the `todo` `Error`, as it does a `panic`.
#[test]
fn todo_with_a_label_is_an_error() {
    codes(
        &body("", "let x = todo(m: \"x\")"),
        &[(Code::TodoLeft, (2, 13)), (Code::LabeledArgument, (2, 18))],
    );
}

/// A line break ends a bare `todo`: `(1)` on the next line is a statement, not its message.
#[test]
fn todo_ends_at_the_line() {
    codes(
        &body("", "todo\n    (1)"),
        &[(Code::TodoLeft, (2, 5)), (Code::UnreachableCode, (3, 5))],
    );
}
