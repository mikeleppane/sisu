//! The front end: from source to the checked program.

use crate::diagnostic::{Diagnostic, DiagnosticKind, Severity, Span};
use crate::{check, last_use, lexer, parser, tir};

/// The source in `bytes`. Bytes that are not UTF-8 are an error at the first invalid one, with
/// the text to render it over: the valid text before it, then U+FFFD in its place, so the
/// position is the bad byte's even when it starts a line or ends the file.
pub(crate) fn decode(bytes: Vec<u8>) -> Result<String, Box<(String, Diagnostic)>> {
    String::from_utf8(bytes).map_err(|e| {
        let valid_up_to = e.utf8_error().valid_up_to();
        let shown = String::from_utf8_lossy(&e.as_bytes()[..valid_up_to]) + "\u{FFFD}";
        let at = Span::new(valid_up_to, shown.len());
        Box::new((
            shown.into_owned(),
            DiagnosticKind::InvalidUtf8 { at }.into(),
        ))
    })
}

/// Lexes, parses, checks and marks last uses in `source`. Returns every diagnostic, sorted by
/// position, and the program unless one is an error. The checker runs only when every item is
/// named: it checks the intact items, and treats a broken one as declared.
pub(crate) fn front_end(source: &str) -> (Option<tir::Program>, Vec<Diagnostic>) {
    let lexed = match lexer::lex(source) {
        Ok(lexed) => lexed,
        Err(d) => return (None, vec![d]),
    };
    let parsed = parser::parse(lexed);
    if !parsed.checkable {
        return (None, parsed.diagnostics);
    }
    let (program, checked) = check::check(&parsed.program);
    let mut diagnostics = parsed.diagnostics;
    diagnostics.extend(checked);
    diagnostics.sort_by_key(|d| d.span().start);
    let failed = diagnostics.iter().any(|d| d.severity() == Severity::Error);
    let program = program.filter(|_| !failed).map(|mut program| {
        last_use::mark(&mut program);
        program
    });
    (program, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::explanations::{apply_fix, fix_edits};
    use crate::diagnostic::{Code, line_col};

    /// The code and `(line, col)` of every diagnostic `src` reports, in order.
    fn codes(src: &str) -> Vec<(Code, (usize, usize))> {
        front_end(src)
            .1
            .iter()
            .map(|d| (d.code(), line_col(src, d.span().start)))
            .collect()
    }

    /// The spec's `a.sisu`.
    const SAMPLE: &str = "fn total(xs: [i64]) -> i64 {
    let sum = 0
    for i in 0..len(xs) {
        sum += xs[i]
    }
    sum
}

fn main() {
    let xs = [1, 2, 3]
    print(totl(xs))
}
";

    #[test]
    fn one_error_per_broken_item() {
        let src = "fn f() {\n    let = 1\n    let = 2\n}\nfn g() {\n    var x = 1\n    var y = 2\n}\nfn main() {\n    let b: bool = 1\n}\n";
        assert_eq!(
            codes(src),
            [
                (Code::Expected, (2, 9)),
                (Code::Var, (6, 5)),
                (Code::Mismatch, (10, 19))
            ]
        );
    }

    #[test]
    fn lexer_error_hides_its_parse_error() {
        let src = "fn main() {\n    let n: i64 = \"three\"\n}\n";
        assert_eq!(codes(src), [(Code::UnexpectedCharacter, (2, 18))]);
    }

    #[test]
    fn spec_sample_reports_three_errors() {
        assert_eq!(
            codes(SAMPLE),
            [
                (Code::MethodAsFunction, (3, 17)),
                (Code::AssignToImmutable, (4, 9)),
                (Code::FunctionNotFound, (11, 11))
            ]
        );
    }

    #[test]
    fn spec_sample_with_quote() {
        let src = SAMPLE.replace(
            "let xs = [1, 2, 3]\n",
            "let xs = [1, 2, 3]\n    let n: i64 = \"three\"\n",
        );
        assert_eq!(
            codes(&src),
            [
                (Code::MethodAsFunction, (3, 17)),
                (Code::AssignToImmutable, (4, 9)),
                (Code::UnexpectedCharacter, (11, 18))
            ]
        );
    }

    #[test]
    fn call_to_broken_function_checks_arguments() {
        let src = "fn f() {\n    let = 1\n}\nfn main() {\n    f(nope)\n}\n";
        assert_eq!(
            codes(src),
            [(Code::Expected, (2, 9)), (Code::BindingNotFound, (5, 7))]
        );
    }

    #[test]
    fn broken_class_poisons_its_uses() {
        // `t` has the broken type, so its method call has a poisoned receiver. That path checks
        // the call's arguments with `stray`, so `other`, a real mistake, is still reported.
        let src = "class Tree {\n    value i64\n    fn m(self, x: i64) {}\n}\nfn main() {\n    let t: Tree = Tree(value: nope)\n    t.m(other)\n}\n";
        assert_eq!(
            codes(src),
            [
                (Code::Expected, (2, 11)),
                (Code::BindingNotFound, (6, 31)),
                (Code::BindingNotFound, (7, 9))
            ]
        );
    }

    #[test]
    fn field_list_error_skips_methods() {
        let src = "class Tree {\n    value i64\n    fn m(self) {\n        let b: bool = 1\n    }\n}\nfn main() {}\n";
        assert_eq!(codes(src), [(Code::Expected, (2, 11))]);
    }

    #[test]
    fn broken_method_only_breaks_itself() {
        let src = "class Counter {\n    n: i64\n    fn broken(self, x: i64) {\n        let = 1\n    }\n    fn typed(self, x: i64) {\n        let b: bool = x\n    }\n}\nfn main() {\n    let c = Counter(n: 0)\n    c.broken(nope)\n    c.typed(1)\n}\n";
        assert_eq!(
            codes(src),
            [
                (Code::Expected, (4, 13)),
                (Code::Mismatch, (7, 23)),
                (Code::BindingNotFound, (12, 14))
            ]
        );
    }

    #[test]
    fn error_before_method_name_breaks_the_class() {
        // `x: true` is a mismatch only if the class is intact.
        let src = "class P {\n    x: i64\n    fn (self) {}\n}\nfn main() {\n    let p: P = P(x: true)\n    print(p.x)\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (3, 8))]);
    }

    #[test]
    fn lexer_error_between_members_breaks_the_class() {
        // `n: true` is a mismatch only if the class is intact.
        let src = "class C {\n    n: i64\n    $\n}\nfn main() {\n    let c = C(n: true)\n}\n";
        assert_eq!(codes(src), [(Code::UnexpectedCharacter, (3, 5))]);
    }

    #[test]
    fn method_without_self_breaks_only_itself() {
        let src = "class C {\n    n: i64\n    fn m() {}\n    fn k(self) {\n        let b: bool = 1\n    }\n}\nfn main() {\n    let c = C(n: 1)\n    c.m()\n}\n";
        assert_eq!(
            codes(src),
            [(Code::MethodNeedsSelf, (3, 8)), (Code::Mismatch, (5, 23))]
        );
    }

    #[test]
    fn broken_class_reports_its_earliest_error() {
        // Two broken methods come before the field list error, which breaks the class.
        let src = "class C {\n    n: i64\n    fn a(self) {\n        let = 1\n    }\n    fn b(self) {\n        let = 2\n    }\n    x i64\n}\nfn main() {}\n";
        assert_eq!(codes(src), [(Code::Expected, (4, 13))]);
    }

    #[test]
    fn skipped_text_before_a_cut_lies_in_the_item() {
        // `Eof` cuts the item off after `->`; the `$` is still its own.
        assert_eq!(codes("fn main() -> $"), [(Code::Expected, (1, 13))]);
        assert_eq!(codes("def total(xs): $"), [(Code::Expected, (1, 1))]);
        // A skip-only line before the next item still belongs to the unnamed segment.
        let src = "def total(xs):\n    return 0\n$\nfn main() {}\n";
        assert_eq!(codes(src), [(Code::Expected, (1, 1))]);
        assert_eq!(codes("def total(xs): x\n$"), [(Code::Expected, (1, 1))]);
        // The class's `}` cuts the method off; the `$` breaks the method, not the class.
        let src = "class C {\n    n: i64\n    fn m(self) -> $\n}\nfn main() {\n    let c = C(n: true)\n}\n";
        assert_eq!(
            codes(src),
            [
                (Code::UnexpectedCharacter, (3, 19)),
                (Code::Mismatch, (6, 18))
            ]
        );
    }

    #[test]
    fn broken_name_poisons_a_built_in() {
        let src = "fn print() {\n    let = 1\n}\nfn main() {\n    print(1, 2)\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (2, 9))]);
        let src = "class i64 {\n    x y\n}\nfn main() {\n    let n: i64 = true\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (2, 7))]);
    }

    #[test]
    fn unnamed_item_stops_the_checker() {
        let src = "def total() {}\nfn main() {\n    let b: bool = 1\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (1, 1))]);
    }

    /// Skipped text outside every item that holds `fn` or `class` may have swallowed an item, so
    /// no name lookup can be trusted.
    #[test]
    fn swallowed_item_stops_the_checker() {
        for src in [
            "\"fn helper() {}\nfn main() {\n    helper()\n}\n",
            "\"class Helper {}\nfn helper() -> Helper {\n    return Helper()\n}\nfn main() {}\n",
            "\"fn main() {}\n",
        ] {
            assert_eq!(codes(src), [(Code::UnexpectedCharacter, (1, 1))], "{src}");
        }
        // Skipped text without either word keeps the checker on.
        for src in [
            "fn f() {} $\nfn main() {\n    let b: bool = 1\n}\n",
            "fn f() {} \"a note\nfn main() {\n    let b: bool = 1\n}\n",
        ] {
            assert_eq!(
                codes(src),
                [
                    (Code::UnexpectedCharacter, (1, 11)),
                    (Code::Mismatch, (3, 19))
                ],
                "{src}"
            );
        }
    }

    /// Text skipped at the start of a top-level line lies in the unnamed segment after it, whose
    /// one error is the skip.
    #[test]
    fn skip_before_an_unnamed_segment_is_its_error() {
        for (src, line) in [
            (
                "# total of the list\nfn main() {\n    let b: bool = 1\n}\n",
                1,
            ),
            ("@decorator\nfn main() {}\n", 1),
            ("#[test]\nfn main() {}\n", 1),
            ("fn main() {}\n# the end\n", 2),
        ] {
            assert_eq!(
                codes(src),
                [(Code::UnexpectedCharacter, (line, 1))],
                "{src}"
            );
        }
    }

    /// A `;` after an item's `}` ends its line, as it does in a block, and the checker still runs.
    #[test]
    fn semicolon_after_an_item() {
        let src = "class C {\n    x: i64\n};\nfn main() {\n    let b: bool = 1\n}\n";
        assert_eq!(
            codes(src),
            [(Code::Semicolon, (3, 2)), (Code::Mismatch, (5, 19))]
        );
        let d = &front_end(src).1[0];
        assert!(fix_edits(d).is_some_and(|(certain, _)| certain));
        let fixed = apply_fix(src, d);
        assert_eq!(codes(&fixed), [(Code::Mismatch, (5, 19))]);
        // Anywhere else, a `;` is not an item.
        assert_eq!(codes(";\nfn main() {}\n"), [(Code::Expected, (1, 1))]);
    }

    #[test]
    fn foreign_function_is_one_error() {
        let src = "def total(xs):\n    s = 0\n    for x in xs:\n        s += x\n    return s\nfn main() {}\n";
        assert_eq!(codes(src), [(Code::Expected, (1, 1))]);
    }

    #[test]
    fn broken_len_is_not_the_idiom() {
        let src = "fn len(xs: [i64]) -> i64 {\n    let = 1\n}\nfn main() {\n    let xs = [1, 2]\n    print(len(xs))\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (2, 9))]);
    }

    #[test]
    fn damaged_name_stops_the_checker() {
        let src = "fn to$tal() {}\nfn main() {\n    total()\n}\n";
        assert_eq!(codes(src), [(Code::UnexpectedCharacter, (1, 6))]);
        let src = "fn $f() {}\nfn main() {\n    f()\n    let b: bool = 1\n}\n";
        assert_eq!(codes(src), [(Code::UnexpectedCharacter, (1, 4))]);
    }

    #[test]
    fn bracket_error_is_alone() {
        let src = "fn main() {\n    let b: bool = 1\n}\nfn f() { print(1 }\n";
        assert_eq!(codes(src), [(Code::UnexpectedClosingBracket, (4, 18))]);
    }

    #[test]
    fn broken_main_raises_no_main_error() {
        let src = "fn f() {}\nfn main() {\n    let = 1\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (3, 9))]);
        let src = "fn f() {}\nfn main(x: i64) -> i64 {\n    let = 1\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (3, 9))]);
    }

    #[test]
    fn broken_field_type_is_never_never_constructed() {
        let src = "class A {\n    b: B\n}\nclass B {\n    a A\n}\nfn main() {}\n";
        assert_eq!(codes(src), [(Code::Expected, (5, 7))]);
    }

    #[test]
    fn warnings_print_beside_syntax_errors() {
        let src = "fn f() {\n    let mut a = 1\n    let = 1\n}\nfn g() {\n    let mut b = 1\n    print(b)\n}\nfn main() {}\n";
        assert_eq!(
            codes(src),
            [(Code::Expected, (3, 9)), (Code::NeverReassigned, (6, 13))]
        );
    }

    #[test]
    fn broken_item_is_never_a_duplicate() {
        let src = "fn total() {\n    let = 1\n}\nfn total() {}\nfn main() {\n    total()\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (2, 9))]);
    }

    #[test]
    fn broken_item_is_a_candidate() {
        let src = "fn total(n: i64) {\n    let = 1\n}\nfn main() {\n    totl(1)\n}\n";
        assert_eq!(
            codes(src),
            [(Code::Expected, (2, 9)), (Code::FunctionNotFound, (5, 5))]
        );
        let (_, diagnostics) = front_end(src);
        assert_eq!(
            fix_edits(&diagnostics[1]),
            Some((false, vec![(49, 53, "total".to_string())]))
        );
    }

    #[test]
    fn line_break_ends_an_item_without_a_body() {
        assert_eq!(codes("fn f()\nfn main() {}\n"), [(Code::Expected, (1, 7))]);
        assert_eq!(
            codes("fn f() $\nfn main() {}\n"),
            [(Code::UnexpectedCharacter, (1, 8))]
        );
    }

    #[test]
    fn unnamed_item_runs_to_the_next_line_item() {
        let src = "def total() {}\nfn main() {\n    let = 1\n}\n";
        assert_eq!(
            codes(src),
            [(Code::Expected, (1, 1)), (Code::Expected, (3, 9))]
        );
        // `fn` within a line does not start an item.
        let src = "pub fn main() {\n    let = 1\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (1, 1))]);
        assert_eq!(
            codes("fn main() {}\nstruct P {}\n"),
            [(Code::Expected, (2, 1))]
        );
    }

    #[test]
    fn lexer_error_after_a_nested_block() {
        let src = "fn f() {\n    if true {\n    }\n    let n: i64 = \"three\"\n}\nfn main() {}\n";
        assert_eq!(codes(src), [(Code::UnexpectedCharacter, (4, 18))]);
    }

    #[test]
    fn lexer_error_breaks_only_its_method() {
        let src = "class C {\n    n: i64\n    fn m(self) {\n        let n: i64 = \"three\"\n    }\n}\nfn main() {\n    let c = C(n: true)\n}\n";
        assert_eq!(
            codes(src),
            [
                (Code::UnexpectedCharacter, (4, 22)),
                (Code::Mismatch, (8, 18))
            ]
        );
    }

    #[test]
    fn broken_method_is_no_field_confusion() {
        let src = "class C {\n    n: i64\n    fn m(self) {\n        let = 1\n    }\n}\nfn main() {\n    let c = C(n: 1)\n    print(c.m)\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (4, 13))]);
    }

    #[test]
    fn broken_name_poisons_the_other_kind() {
        // Functions and classes share a namespace: the broken function `Tree` poisons the type.
        let src = "class Tree {\n    n: i64\n}\nfn Tree() {\n    let = 1\n}\nfn main() {\n    let t: Tree = 1\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (5, 9))]);
    }
}
