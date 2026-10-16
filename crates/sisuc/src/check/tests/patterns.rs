//! `_` patterns: `let _ = e` and `for _ in ...` bind no name.

use super::*;

#[test]
fn let_underscore_drops_at_once() {
    let printed =
        lowered("fn make() -> [i64] { [1] }\nfn main() {\n    let _ = make()\n    print(1)\n}");
    assert!(printed.contains("(call make) (print 1)"), "{printed}");
    assert!(!printed.contains("(let"), "{printed}");
}

/// A `let` has no value, whatever it lowers to.
#[test]
fn let_underscore_ends_a_block_without_a_value() {
    clean("fn make() -> [i64] { [1] }\nfn main() {\n    let _ = make()\n}");
    clean(
        "fn make() -> [i64] { [1] }\nfn f() -> i64 {\n    let _ = make()\n    1\n}\nfn main() {}",
    );
    codes(
        "fn make() -> [i64] { [1] }\nfn f() -> [i64] {\n    let _ = make()\n}\nfn main() {}",
        &[(Code::BodyHasNoValue, (4, 1))],
    );
}

#[test]
fn let_underscore_checks_its_type_and_value() {
    clean("fn main() {\n    let _: i64 = 1\n}");
    codes(
        "fn main() {\n    let _: bool = 1\n}",
        &[(Code::Mismatch, (2, 19))],
    );
    codes(
        "fn main() {\n    let _ = nope\n}",
        &[(Code::BindingNotFound, (2, 13))],
    );
}

#[test]
fn let_underscore_declares_no_name() {
    codes(
        "fn main() {\n    let _ = 1\n    let _ = 2\n    print(1)\n}",
        &[],
    );
}

#[test]
fn if_let_and_while_let_take_underscore() {
    clean("fn f(n: i64?) {\n    if let _ = n {}\n    while let _ = n {}\n}\nfn main() {}");
    codes(
        "fn main() {\n    if let _ = 5 {}\n}",
        &[(Code::UnwrapNeedsOptional, (2, 16))],
    );
}

/// The declarations the `match` tests share: lines 1 to 12.
const DECLS: &str = "enum Expr {
    Num(i64)
    Add(i64, i64)
}
class C { mut count: i64 }
enum State {
    Idle
    Running(C)
}
enum Shape { Circle(i64) }
enum Opt { V(i64?) }
fn main() {}";

/// `body` on line 14, in a function of line 13 whose parameters are of the shared types.
fn in_fn(body: &str) -> String {
    format!("{DECLS}\nfn t(e: Expr, s: State, p: C, maybe: Expr?, o: Opt, n: i64) {{\n{body}\n}}")
}

/// `body` as a `match` on `scrutinee` whose arm `arm` is on line 15, then `_ => 0`.
fn arm_on_15(scrutinee: &str, arm: &str) -> String {
    in_fn(&format!(
        "    let _ = match {scrutinee} {{\n        {arm}\n        _ => 0\n    }}"
    ))
}

#[test]
fn match_lowers() {
    let rows = [
        (
            "fn f(e: Expr) -> i64 {\n    match e {\n        Num(n) => n\n        Add(a, b) => 0\n    }\n}",
            "(match e#0 (arm (Num n#1) (block n#1)) (arm (Add a#2 b#3) (block 0)))",
        ),
        (
            "fn f(e: Expr) -> i64 {\n    match e {\n        Expr.Num(n) => n\n        Add(_, _) => 1\n    }\n}",
            "(match e#0 (arm (Num n#1) (block n#1)) (arm (Add _ _) (block 1)))",
        ),
        (
            "fn f(s: State) -> i64 {\n    match s {\n        Idle => 0\n        Running(c) => c.count\n    }\n}",
            "(match s#0 (arm Idle (block 0)) (arm (Running c#1) (block (. c#1 count))))",
        ),
        // `if let` on an enum is a `match` of two arms, the second `_`.
        (
            "fn f(s: State) -> i64 {\n    if let Running(c) = s { c.count } else { 0 }\n}",
            "(match s#0 (arm (Running c#1) (block (. c#1 count))) (arm _ (block 0)))",
        ),
        // Without an `else`, the `_` arm is empty.
        (
            "fn f(s: State) {\n    if let Running(c) = s {\n        print(c.count)\n    }\n}",
            "(match s#0 (arm (Running c#1) (block (print (. c#1 count)))) (arm _ (block)))",
        ),
        // `while let` on an enum loops over a `match` whose `_` arm breaks.
        (
            "enum L {\n    Nil\n    Cons(i64, L)\n}\nfn f(start: L) -> i64 {\n    let mut l = start\n    let mut total = 0\n    while let Cons(h, t) = l {\n        total += h\n        l = t\n    }\n    total\n}",
            "(loop (block (match l#1 (arm (Cons h#3 t#4) (block (= total#2 (+ total#2 h#3)) (= l#1 t#4))) (arm _ (block (break))))))",
        ),
    ];
    for (function, part) in rows {
        contains(&format!("{DECLS}\n{function}"), &[part]);
    }
}

/// The type of the local `v` in the function `t` of `src`, which checks clean.
fn type_of_v(src: &str) -> String {
    let (program, ds) = checked(src);
    assert!(ds.is_empty(), "{src}: {ds:?}");
    let program = program.expect("the row should check clean");
    let t = program
        .functions
        .iter()
        .find(|f| f.name == "t")
        .expect("the row should define `t`");
    let v = t
        .locals
        .iter()
        .find(|l| l.name.as_deref() == Some("v"))
        .expect("`t` should bind `v`");
    v.ty.to_string()
}

#[test]
fn match_join() {
    // Each row: the arms' values, in order, and the type they join to.
    let rows: [(&[&str], &str); 10] = [
        (&["1", "return 2", "3"], "i64"),
        // A `never` arm sets no type for the arms after it: the `None` expects `i64?`.
        (&["return 1", "1", "None"], "i64?"),
        (&["1", "b"], "u8"),
        (&["b", "1"], "u8"),
        (&["None", "1"], "i64?"),
        (&["1", "None"], "i64?"),
        (&["[]", "xs"], "[string]"),
        (&["xs", "[]"], "[string]"),
        (&["b", "b", "None"], "u8?"),
        // A block arm ranks by the value it ends in.
        (&["{ None }", "1"], "i64?"),
    ];
    let patterns = ["Num(_)", "Add(_, _)", "_"];
    for (values, want) in rows {
        let arms: String = patterns[3 - values.len()..]
            .iter()
            .zip(values)
            .map(|(p, v)| format!("        {p} => {v}\n"))
            .collect::<Vec<_>>()
            .concat();
        let src = format!(
            "{DECLS}\nfn t(e: Expr, b: u8, xs: [string]) -> i64 {{\n    let v = match e {{\n{arms}    }}\n    0\n}}"
        );
        assert_eq!(type_of_v(&src), want, "{values:?}");
    }
    // A failed arm sets no type for the arms after it: `b`, checked after it, does, so the `1`
    // expects `u8`.
    codes(
        &format!(
            "{DECLS}\nfn t(e: Expr, b: u8) {{\n    let _ = match e {{\n        Num(_) => nope\n        Add(_, _) => b\n        _ => 1\n    }}\n}}"
        ),
        &[(Code::BindingNotFound, (15, 19))],
    );
    // A failed arm after one of a known type keeps that type for the arms after it.
    codes(
        &format!(
            "{DECLS}\nfn t(e: Expr, b: u8) {{\n    let _ = match e {{\n        Num(_) => b\n        Add(_, _) => nope\n        _ => 1\n    }}\n}}"
        ),
        &[(Code::BindingNotFound, (16, 22))],
    );
    // The first arm of a known type anchors the rest, not a later one: the `None` expects `u8?`.
    codes(
        &format!(
            "{DECLS}\nfn t(e: Expr, b: u8) {{\n    let _ = match e {{\n        Num(_) => b\n        Add(_, _) => print(1)\n        _ => None\n    }}\n}}"
        ),
        &[(Code::NoValue, (16, 22))],
    );
    // A `unit` arm checked first anchors only until an arm of another type: the `None` expects
    // `u8?`.
    codes(
        &format!(
            "{DECLS}\nfn t(e: Expr, b: u8) {{\n    let _ = match e {{\n        Num(_) => print(1)\n        Add(_, _) => b\n        _ => None\n    }}\n}}"
        ),
        &[(Code::NoValue, (15, 19))],
    );
    // Arms that all leave make a `never` `match`.
    clean(&format!(
        "{DECLS}\nfn t(e: Expr) -> i64 {{\n    match e {{\n        Num(_) => return 1\n        _ => return 2\n    }}\n}}"
    ));
    // The `T` arms of an optional join are wrapped.
    contains(
        &format!(
            "{DECLS}\nfn t(e: Expr) -> i64? {{\n    match e {{\n        Num(_) => None\n        _ => 1\n    }}\n}}"
        ),
        &["(arm _ (block (wrap 1)))"],
    );
    // Arms that do not join: the later one is the mistake, against the earlier.
    codes(
        &arm_on_15("e", "Num(_) => \"a\""),
        &[(Code::Mismatch, (16, 14))],
    );
    // With a type expected, the arm that does not fit it is.
    codes(
        &in_fn("    let _: i64 = match e {\n        Num(_) => \"a\"\n        _ => 0\n    }"),
        &[(Code::Mismatch, (15, 19))],
    );
    // An arm with no value is the mistake.
    codes(
        &arm_on_15("e", "Num(_) => print(1)"),
        &[(Code::NoValue, (15, 19))],
    );
}

#[test]
fn pattern_names() {
    let decls = "const Idle = 0\nconst LIMIT = 9\nconst limit = 9";
    // Each row: the scrutinee, the arm on line 18, and the code it reports at column 9.
    let rows = [
        ("s", "Stopped => 1", Some(Code::UnknownVariant)),
        ("n", "Stopped => 1", Some(Code::NotVariantHere)),
        // The name still binds, so the body reports nothing more.
        ("n", "Count => Count + 1", Some(Code::NotVariantHere)),
        // That binding is not reported again for taking an enum's name.
        ("n", "State => 1", Some(Code::NotVariantHere)),
        ("n", "LIMIT => 1", Some(Code::ConstantInPattern)),
        ("n", "limit => 1", Some(Code::ConstantInPattern)),
        // A variant of the enum comes before the constant of its name.
        ("s", "Idle => 1", None),
        ("s", "State.Idle => 1", None),
    ];
    for (scrutinee, arm, code) in rows {
        let src = format!("{decls}\n{}", arm_on_15(scrutinee, arm));
        let expected: Vec<_> = code.into_iter().map(|c| (c, (18, 9))).collect();
        codes(&src, &expected);
    }
    contains(
        &format!("{decls}\n{}", arm_on_15("s", "Idle => 1")),
        &["(arm Idle (block 1))"],
    );
}

#[test]
fn pattern_errors() {
    // Each row: the scrutinee, the arm on line 15, and the code it reports there.
    let rows = [
        ("e", "Shape.Circle(r) => 1", Code::Mismatch, 9),
        ("e", "Num(Num(m)) => 1", Code::Mismatch, 13),
        ("n", "Num(m) => 1", Code::Mismatch, 9),
        ("n", "None => 1", Code::Mismatch, 9),
        ("e", "Add(x) => 1", Code::SubpatternCount, 9),
        ("e", "Add(x, x) => 1", Code::BoundTwice, 16),
        ("s", "Running(Idle) => 1", Code::NotVariantHere, 17),
        ("s", "Running(Num(m)) => 1", Code::OnlyNamePosition, 17),
        ("o", "V(None) => 1", Code::OnlyNamePosition, 11),
        ("o", "V(Num(m)) => 1", Code::OnlyNamePosition, 11),
        ("s", "Idle() => 1", Code::VariantForm, 13),
        ("e", "Expr.Num => 1", Code::VariantForm, 14),
        ("e", "Num => 1", Code::VariantForm, 9),
        ("e", "Nmu(m) => 1", Code::UnknownVariant, 9),
        ("e", "Exr.Num(m) => 1", Code::UnknownType, 9),
        ("n", "Ok => 1", Code::BuiltInName, 9),
        ("e", "Num(x, y) => 1", Code::SubpatternCount, 9),
        // A failed pattern binds each name once, so `x` is not bound twice.
        ("e", "Mul(x, x) => 1", Code::UnknownVariant, 9),
        // A body fails on its own, after a pattern that checks.
        ("e", "Num(m) => nope", Code::BindingNotFound, 19),
        // The fields after a failed one are checked too, so their names are bound.
        ("e", "Add(Nmu, b) => b", Code::NotVariantHere, 13),
        ("e", "Add(Num(x, y), m) => m", Code::Mismatch, 13),
        ("e", "Add(None, y) => y", Code::Mismatch, 13),
    ];
    for (scrutinee, arm, code, col) in rows {
        codes(&arm_on_15(scrutinee, arm), &[(code, (15, col))]);
    }
    // Each row: the statement on line 14, and the code it reports there.
    let rows = [
        ("    let _ = match p {}", Code::CannotMatch, 19),
        ("    if let m = 5 {}", Code::UnwrapNeedsOptional, 16),
        ("    if let Num(m) = maybe {}", Code::OnlyNamePosition, 12),
        ("    if let None = maybe {}", Code::OnlyNamePosition, 12),
        ("    while let Num(m) = 5 {}", Code::UnwrapNeedsOptional, 24),
        ("    if let m = p {}", Code::UnwrapNeedsOptional, 16),
        ("    while let Nmu(m) = e {}", Code::UnknownVariant, 15),
        (
            "    while let Add(Q, y) = e { print(y) }",
            Code::NotVariantHere,
            19,
        ),
    ];
    for (stmt, code, col) in rows {
        codes(&in_fn(stmt), &[(code, (14, col))]);
    }
    // A name bound twice that is no variant is reported each time, as no variant only.
    codes(
        &arm_on_15("e", "Add(Q, Q) => 1"),
        &[
            (Code::NotVariantHere, (15, 13)),
            (Code::NotVariantHere, (15, 16)),
        ],
    );
    // A payload of an unknown type matches any sub-pattern, and binds its names as errors.
    codes(
        &format!(
            "{DECLS}\nenum W {{ Wrap(Nope) }}\nfn f(w: W) -> i64 {{\n    match w {{\n        Wrap(Num(m)) => m + 1\n    }}\n}}"
        ),
        &[(Code::UnknownType, (13, 15))],
    );
    // The names of a failed pattern are bound as errors, so the body reports nothing more.
    codes(
        &arm_on_15("e", "Add(x) => x + 1"),
        &[(Code::SubpatternCount, (15, 9))],
    );
}

/// A pattern of another enum is reported from its qualifier on; the fixtures pin the other
/// pattern spans.
#[test]
fn pattern_spans() {
    diagnostic(
        &arm_on_15("e", "Shape.Circle(r) => 1"),
        Code::Mismatch,
        (15, 9, "Shape.Circle(r)"),
        &[],
    );
}

#[test]
fn variant_form_fix_is_safe_only_when_empty() {
    let src = arm_on_15("s", "Idle() => 1");
    let d = diagnostic(&src, Code::VariantForm, (15, 13, "()"), &[]);
    assert_eq!(
        fixes(&src, &d),
        [(true, vec![(15, 13, "()", String::new())])]
    );
    fix_removes_its_code(&src, &d);
    let src = arm_on_15("s", "Idle(// keep\n        ) => 1");
    let text = "(// keep\n        )";
    let d = diagnostic(&src, Code::VariantForm, (15, 13, text), &[]);
    assert_eq!(
        fixes(&src, &d),
        [(false, vec![(15, 13, text, String::new())])]
    );
}

#[test]
fn pattern_bindings_are_immutable() {
    // The secondary span is the binding, which the label says `match` bound.
    diagnostic(
        &format!(
            "{DECLS}\nfn t(e: Expr) {{\n    match e {{\n        Add(w, h) => w = 5\n        _ => {{}}\n    }}\n}}"
        ),
        Code::AssignToImmutable,
        (15, 22, "w = 5"),
        &[(15, 13)],
    );
}

#[test]
fn last_use_through_match() {
    let src = format!(
        "{DECLS}\nfn t(s: State, d: C) -> C {{\n    match s {{\n        Running(c) => c\n        Idle => d\n    }}\n}}"
    );
    let (program, ds) = crate::front_end::front_end(&src);
    assert!(ds.is_empty(), "{ds:?}");
    let printed = program.expect("the source should check clean").to_string();
    let part =
        "(match (move s#0) (arm (Running c#2) (block (move c#2))) (arm Idle (block (move d#1))))";
    assert!(printed.contains(part), "{part} in {printed}");
    // A local a later arm reads is live in the arms before it and at the test: the scrutinee
    // is not moved into the `match`.
    let src = format!(
        "{DECLS}\nfn t(s: State) -> State {{\n    match s {{\n        Running(_) => State.Idle\n        Idle => s\n    }}\n}}"
    );
    let (program, ds) = crate::front_end::front_end(&src);
    assert!(ds.is_empty(), "{ds:?}");
    let printed = program.expect("the source should check clean").to_string();
    for part in ["(match s#0 ", "(arm Idle (block (move s#0)))"] {
        assert!(printed.contains(part), "{part} in {printed}");
    }
    // An arm's bindings are dead before its test, so the loop moves `l` into the `match` and
    // `t` into `l`.
    let src = "enum L {\n    Cons(i64, L)\n    Nil\n}\nfn main() {}\nfn t(l0: L) {\n    let mut l = l0\n    while let Cons(h, t) = l {\n        print(h)\n        l = t\n    }\n}";
    let (program, ds) = crate::front_end::front_end(src);
    assert!(ds.is_empty(), "{ds:?}");
    let printed = program.expect("the source should check clean").to_string();
    for part in ["(match (move l#1)", "(= l#1 (move t#3))"] {
        assert!(printed.contains(part), "{part} in {printed}");
    }
}

#[test]
fn match_spills() {
    // A counted argument before a `match` that exits is spilled, as before an `if`.
    let src = format!(
        "{DECLS}\nfn g(xs: [i64], k: i64) -> i64 {{ k }}\nfn f(xs: [i64], e: Expr) -> i64 {{\n    let _ = g(xs, match e {{\n        Num(_) => return 1\n        _ => 2\n    }})\n    0\n}}"
    );
    contains(
        &src,
        &[
            "(let #3 xs#0) (let #4 (match e#1 (arm (Num _) (block (return 1))) (arm _ (block 2)))) (call g #3 #4)",
        ],
    );
}

/// `i64(...)` around a `match` that is already an `i64` keeps the `match` grouped unless
/// parentheses already group it.
#[test]
fn already_converted_match_stays_grouped() {
    let src = in_fn("    let _ = i64(match e {\n        _ => 1\n    }) + 1");
    let d = diagnostic(
        &src,
        Code::AlreadyConverted,
        (14, 17, "match e {\n        _ => 1\n    }"),
        &[],
    );
    let fixed = applied(&src, &d);
    assert!(fixed.contains("let _ = (match e {"), "{fixed}");
    let src = in_fn("    let _ = i64((match e {\n        _ => 1\n    })) + 1");
    let d = diagnostic(
        &src,
        Code::AlreadyConverted,
        (14, 17, "(match e {\n        _ => 1\n    })"),
        &[],
    );
    let fixed = applied(&src, &d);
    assert!(fixed.contains("let _ = (match e {"), "{fixed}");
}

/// A scrutinee `match` cannot take fails the `match` as a failed condition fails an `if`: its
/// arms' `never` stays, and any other type becomes `Error`. With no arms, a scrutinee of a known
/// type makes it `never`, and an unknown one `Error`.
#[test]
fn failed_scrutinee_keeps_never() {
    codes(
        &format!(
            "{DECLS}\nfn f(p: C) -> i64 {{\n    match p {{\n        _ => return 1\n    }}\n    print(3)\n}}"
        ),
        &[
            (Code::CannotMatch, (14, 11)),
            (Code::UnreachableCode, (17, 5)),
        ],
    );
    codes(
        &in_fn("    let x = match p {\n        _ => 1\n    }\n    let _: string = x"),
        &[(Code::CannotMatch, (14, 19))],
    );
    codes(
        &in_fn("    match n {}\n    print(3)"),
        &[(Code::UnreachableCode, (15, 5))],
    );
    codes(
        &in_fn("    match nope {}\n    print(3)"),
        &[(Code::BindingNotFound, (14, 11))],
    );
}
