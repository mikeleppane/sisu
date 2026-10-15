//! Enum declarations: variants and payloads, the reserved names, which enums can be built, the
//! boxes that break a cycle, and which enums are counted.

use std::fmt::Write as _;

use super::*;
use crate::front_end::front_end;

/// `decls` with an empty `main`.
fn with_main(decls: &str) -> String {
    format!("{decls}\nfn main() {{}}")
}

/// The declarations, and the codes and positions they report.
type Row<'a> = (&'a str, &'a [(Code, (usize, usize))]);

#[test]
fn enum_declarations_lower() {
    let rows = [
        (
            "enum Shape {\n    Circle(radius: i64)\n    Rect(w: i64, h: i64)\n}",
            "(enum Shape (Circle i64) (Rect i64 i64))",
        ),
        (
            "class C { mut n: i64 }\nenum State {\n    Idle\n    Running(C)\n}",
            "(enum State Idle (Running C))",
        ),
        (
            "enum Expr {\n    Num(i64)\n    Add(Expr, Expr)\n}",
            "(enum Expr (Num i64) (Add (box Expr) (box Expr)))",
        ),
        // Each payload owns its labels.
        (
            "enum Shape {\n    Rect(w: i64, h: i64)\n    Square(w: i64)\n}",
            "(enum Shape (Rect i64 i64) (Square i64))",
        ),
    ];
    for (decls, part) in rows {
        contains(&with_main(decls), &[part]);
    }
}

#[test]
fn boxes_follow_components() {
    // Each row: the declarations, the printed enums, and each enum's cycle, by enum.
    let rows: [(&str, &[&str], &[bool]); 4] = [
        (
            "enum A { X(B) }\nenum B {\n    Y(A)\n    Z\n}",
            &["(enum A (X (box B)))", "(enum B (Y (box A)) Z)"],
            &[true, true],
        ),
        (
            "enum L {\n    Nil\n    Cons(i64, L?)\n}",
            &["(enum L Nil (Cons i64 (box L?)))"],
            &[true],
        ),
        (
            "enum T {\n    Leaf\n    Node([T])\n}",
            &["(enum T Leaf (Node [T]))"],
            &[false],
        ),
        (
            "enum P { Q(Inner) }\nenum Inner { R(i64) }",
            &["(enum P (Q Inner))", "(enum Inner (R i64))"],
            &[false, false],
        ),
    ];
    for (decls, parts, cycles) in rows {
        let src = with_main(decls);
        contains(&src, parts);
        let program = checked(&src).0.expect("the row should check clean");
        let found: Vec<bool> = program.enums.iter().map(|e| e.cycle.is_some()).collect();
        assert_eq!(found, cycles, "{decls}");
    }
    // `A` and `B` share one component, so one cycle.
    let program = checked(&with_main(rows[0].0))
        .0
        .expect("A and B should check clean");
    assert_eq!(program.enums[0].cycle, program.enums[1].cycle);
}

#[test]
fn enum_countedness() {
    // Each row: the declarations of `E`, and whether `E` is counted.
    let rows = [
        ("enum E {\n    A\n    B\n}", false),
        ("enum E {\n    A(i64, bool)\n    B(char)\n}", false),
        ("enum E {\n    A(string)\n}", true),
        ("class C { n: i64 }\nenum E {\n    A(C)\n}", true),
        ("enum E {\n    A\n    B(E)\n}", true),
        ("enum S {\n    A(string)\n}\nenum E {\n    B(S)\n}", true),
        ("enum S {\n    A(i64)\n}\nenum E {\n    B(S)\n}", false),
    ];
    for (decls, counted) in rows {
        let src = with_main(&format!(
            "{decls}\nfn f(e: E) {{\n    let a = e\n    let b = e\n    print(1)\n}}"
        ));
        let (program, ds) = front_end(&src);
        assert!(ds.is_empty(), "{ds:?}");
        let printed = program.expect("the row should check clean").to_string();
        let second = if counted {
            "(let b#2 (move e#0))"
        } else {
            "(let b#2 e#0)"
        };
        assert!(printed.contains(second), "{decls}: {second} in {printed}");
    }
}

#[test]
fn enum_declaration_errors() {
    let rows: [(&str, _); 9] = [
        ("enum D {\n    north\n}", (Code::VariantCase, (2, 5))),
        (
            "enum M {\n    Move(i64, to: i64)\n}",
            (Code::MixedLabels, (2, 15)),
        ),
        ("enum Empty {}", (Code::EmptyEnum, (1, 6))),
        ("enum E {\n    A\n    A\n}", (Code::DefinedTwice, (3, 5))),
        (
            "enum E {\n    V(x: i64, x: i64)\n}",
            (Code::DefinedTwice, (2, 15)),
        ),
        ("enum E {\n    V(Nope)\n}", (Code::UnknownType, (2, 7))),
        ("enum E { A }\nclass E {}", (Code::DefinedTwice, (2, 7))),
        ("class E {}\nenum E { A }", (Code::DefinedTwice, (2, 6))),
        (
            "enum E {\n    A\n    fn A(self) {}\n}",
            (Code::DefinedTwice, (3, 8)),
        ),
    ];
    for (decls, expected) in rows {
        codes(&with_main(decls), &[expected]);
    }
}

#[test]
fn reserved_names() {
    let rows: [Row<'_>; 8] = [
        ("class Result {}", &[(Code::BuiltInName, (1, 7))]),
        ("fn Option() {}", &[(Code::BuiltInName, (1, 4))]),
        ("enum E {\n    Ok(i64)\n}", &[(Code::BuiltInName, (2, 5))]),
        (
            "fn g() {\n    let Some = 1\n}",
            &[(Code::BuiltInName, (2, 9))],
        ),
        ("fn f(Err: i64) {}", &[(Code::BuiltInName, (1, 6))]),
        ("class C {\n    Ok: i64\n    fn Some(self) {}\n}", &[]),
        // The item and the binding are still declared, so their uses do not cascade.
        (
            "class Result {}\nfn f(r: Result) {}",
            &[(Code::BuiltInName, (1, 7))],
        ),
        (
            "fn g() {\n    let Err = 1\n    print(Err)\n}",
            &[(Code::BuiltInName, (2, 9))],
        ),
    ];
    for (decls, expected) in rows {
        codes(&with_main(decls), expected);
    }
}

#[test]
fn enum_named_local() {
    let rows = [
        (
            "enum Shape { Dot }\nfn g() {\n    let Shape = 1\n    print(Shape)\n}",
            (3, 9),
        ),
        ("enum Shape { Dot }\nfn f(Shape: i64) {}", (2, 6)),
        (
            "enum Shape { Dot }\nfn g() {\n    for Shape in [1] {}\n}",
            (3, 9),
        ),
    ];
    for (decls, at) in rows {
        codes(&with_main(decls), &[(Code::EnumNamedLocal, at)]);
    }
}

#[test]
fn never_built() {
    let rows: [Row<'_>; 11] = [
        (
            "enum Bad { Wrap(Bad) }",
            &[(Code::NeverConstructed, (1, 6))],
        ),
        // A field of an unknown type has its own error, and the chain walks past it.
        (
            "enum Bad { Wrap(Nope, Bad) }",
            &[
                (Code::NeverConstructed, (1, 6)),
                (Code::UnknownType, (1, 17)),
            ],
        ),
        // The chain walks past fields of other types, and past a type it reached already.
        (
            "enum Bad { Wrap(i64, Fine, Bad) }\nenum Fine { A }",
            &[(Code::NeverConstructed, (1, 6))],
        ),
        (
            "enum D { Q(K, K) }\nclass K {\n    d: D\n    x: Nope\n}",
            &[
                (Code::NeverConstructed, (1, 6)),
                (Code::UnknownType, (4, 8)),
            ],
        ),
        (
            "enum D { Q(K) }\nclass K { d: D }",
            &[(Code::NeverConstructed, (1, 6))],
        ),
        (
            "enum C { Z(Bad) }\nenum Bad { Wrap(Bad) }",
            &[(Code::NeverConstructed, (2, 6))],
        ),
        // An enum whose error is reported already is not reported again.
        ("enum E {}\nenum F { G(E) }", &[(Code::EmptyEnum, (1, 6))]),
        // The empty enum's node lies past the classes and the enums before it.
        (
            "class C { n: i64 }\nenum A { X }\nenum E {}\nenum F { G(E) }",
            &[(Code::EmptyEnum, (3, 6))],
        ),
        ("enum A { X(B) }\nenum B {\n    Y(A)\n    Z\n}", &[]),
        ("enum L { Cons([L]) }", &[]),
        ("enum L { Cons(L?) }", &[]),
    ];
    for (decls, expected) in rows {
        codes(&with_main(decls), expected);
    }
}

#[test]
fn variant_case_fix() {
    // Each row: the variant, and the name its fix gives, if any.
    let rows = [
        ("north", Some("North")),
        ("_foo", Some("_Foo")),
        ("_1x", Some("_1X")),
        ("_1", None),
    ];
    for (variant, fixed) in rows {
        let src = with_main(&format!("enum D {{\n    {variant}\n}}"));
        let d = diagnostic(&src, Code::VariantCase, (2, 5, variant), &[]);
        let expected: Vec<FixAt<'_>> = fixed
            .map(|f| (false, vec![(2, 5, variant, f.to_owned())]))
            .into_iter()
            .collect();
        assert_eq!(fixes(&src, &d), expected, "{variant}");
    }
}

#[test]
fn long_declaration_chain_checks() {
    // `components` keeps its own stack: a walk that recursed once per enum would need far more
    // than the test thread's stack for this chain.
    const N: usize = 50_000;
    let mut decls = String::new();
    for i in 0..N - 1 {
        writeln!(decls, "enum E{i} {{ A(E{}) }}", i + 1)
            .expect("writing to a string should not fail");
    }
    write!(decls, "enum E{} {{ A }}", N - 1).expect("writing to a string should not fail");
    codes(&with_main(&decls), &[]);
}

#[test]
fn class_cycle_not_repeated() {
    codes(
        &with_main("class N { next: N }\nenum E { V(N) }"),
        &[(Code::NeverConstructed, (1, 11))],
    );
    // `N` and `E` need each other, but only through `N`'s own cycle.
    codes(
        &with_main("class N {\n    next: N\n    e: E\n}\nenum E { V(N) }"),
        &[(Code::NeverConstructed, (2, 5))],
    );
}
