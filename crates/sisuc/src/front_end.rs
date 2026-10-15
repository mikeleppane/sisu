//! The front end: from source to the checked program.

use crate::diagnostic::{Diagnostic, DiagnosticKind, Severity, Span};
use crate::{check, last_use, lexer, parser, tir};

/// The longest source in bytes: 1 GiB. A `Span` stores its offsets as `u32`s, and two texts
/// longer than the source get spans too:
/// - `decode`'s shown text, the valid prefix and one U+FFFD: at most `MAX_SOURCE + 3` bytes;
/// - a fix applied to the source: `sound_fixes` applies each fix alone and lexes the result
///   again. Only `diagnostic::registry` builds fixes. One writes either at most 11 bytes of its
///   own text (`char(i64(` and `))` around an operand; `let _ = ` is 8; a typed literal is at
///   most `: char` and a 3-byte literal, in place of a call of at least 5 bytes), or one name
///   and at most 4 bytes: a declared field (`{field}: `), a similar name, or the array method a
///   call names (`(` and `{close}.{name}(`). That name is a built-in one or also stands
///   elsewhere in the source, in its declaration or in the call the fix rewrites, so a fix
///   inserts at most source-sized text and the fixed text is at most `2 * MAX_SOURCE` bytes.
pub(crate) const MAX_SOURCE: usize = 1 << 30;

const _: () = assert!(2 * MAX_SOURCE + 3 <= u32::MAX as usize);

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
    let (program, checked) = check::check(&parsed.program, source);
    let mut diagnostics = parsed.diagnostics;
    diagnostics.extend(checked);
    diagnostics.sort_by_key(|d| d.span().start());
    let failed = diagnostics.iter().any(|d| d.severity() == Severity::Error);
    let program = program.filter(|_| !failed).map(|mut program| {
        last_use::mark(&mut program);
        program
    });
    (program, diagnostics)
}

/// `diagnostics` without each fix that leaves `source` lexing or parsing worse than it does:
/// whatever the rule behind a fix, a fix that is shown never breaks the syntax.
pub(crate) fn sound_fixes(source: &str, diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    let mut sound = diagnostics.to_vec();
    // Judging a fix lexes and parses `source` again, so skip that when no diagnostic has one.
    if sound.iter().any(|d| !d.fixes().is_empty()) {
        let before = syntax_errors(source);
        for d in &mut sound {
            d.retain_fixes(|fix| syntax_errors(&fix.apply(source)) <= before);
        }
    }
    sound
}

/// How badly `source` fails to lex or to parse: a lexer error, which stops the parser, is worse
/// than any number of parser errors.
fn syntax_errors(source: &str) -> (bool, usize) {
    match lexer::lex(source) {
        Err(_) => (true, 0),
        Ok(lexed) => {
            let parsed = parser::parse(lexed);
            let errors = parsed.diagnostics.iter();
            (
                false,
                errors.filter(|d| d.severity() == Severity::Error).count(),
            )
        }
    }
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
            .map(|d| (d.code(), line_col(src, d.span().start())))
            .collect()
    }

    /// Whether each fix of `d` is kept by `sound_fixes` over `src`.
    fn kept(src: &str, d: &Diagnostic) -> Vec<bool> {
        let sound = sound_fixes(src, std::slice::from_ref(d));
        let kept = sound[0].fixes();
        d.fixes().iter().map(|f| kept.contains(f)).collect()
    }

    /// A fix that leaves the source lexing and parsing is shown; one that breaks either is not,
    /// whatever its diagnostic. A fix of a syntax error is held only to not making things worse.
    #[test]
    fn a_fix_that_breaks_the_syntax_is_dropped() {
        let src = "fn f(s: string, n: i64) {\n    print(s + n)\n}\nfn main() {}\n";
        let [d] = <[Diagnostic; 1]>::try_from(front_end(src).1).expect("one diagnostic");
        assert_eq!(kept(src, &d), [true]);
        // A `\\` before the `(` of `print(` does not lex.
        let at = Span::new(35, 36);
        assert_eq!(&src[at.range()], "(");
        assert_eq!(
            kept(src, &DiagnosticKind::BraceInString { at }.into()),
            [false]
        );
        // Two lexer errors: the fix of the first leaves the second, which is no worse.
        let src = "fn main() {\n    print(\"}\")\n    print(\"}\")\n}\n";
        let [d] = <[Diagnostic; 1]>::try_from(front_end(src).1).expect("one diagnostic");
        assert_eq!(kept(src, &d), [true]);
        // Two parser errors: likewise.
        let src = "fn main() {\n    let mut _ = 1\n}\nfn g() {\n    let mut _ = 2\n}\n";
        let ds = front_end(src).1;
        assert_eq!(ds.len(), 2, "{ds:?}");
        for d in &ds {
            assert_eq!(kept(src, d), [true]);
        }
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
        let src = "fn main() {\n    let n: i64 = $\n}\n";
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

    /// An unclosed string breaks `main`, so the lexer's error stands for it and `totl` goes
    /// unreported.
    #[test]
    fn spec_sample_with_quote() {
        let src = SAMPLE.replace(
            "let xs = [1, 2, 3]\n",
            "let xs = [1, 2, 3]\n    let n: i64 = \"three\n",
        );
        assert_eq!(
            codes(&src),
            [
                (Code::MethodAsFunction, (3, 17)),
                (Code::AssignToImmutable, (4, 9)),
                (Code::LiteralNotClosed, (11, 18))
            ]
        );
    }

    /// A bad spec is reported once and breaks nothing around it; a spec that a `"` cuts off is
    /// not read, and the unclosed literal is its function's one error.
    #[test]
    fn format_spec_recovery() {
        use Code::{
            EmptyInterpolation, Expected, LiteralNotClosed, Mismatch, UnsupportedFormatSpec,
        };
        for (src, expected) in [
            (
                "fn main() {\n    let w = 3\n    print(\"{w:>{w}}\")\n}\n",
                vec![(UnsupportedFormatSpec, (3, 14))],
            ),
            (
                "fn main() {\n    let x = 1.5\n    let s = \"{x:>8\"\n}\nfn f() {\n    let y: i64 = \"a\"\n}\n",
                vec![(LiteralNotClosed, (3, 13)), (Mismatch, (6, 18))],
            ),
            (
                "fn main() {\n    let x = 1\n    print(\"{x:+\" + \"abc}\")\n}\n",
                vec![(LiteralNotClosed, (3, 11))],
            ),
            // A spec with no expression before it, as Rust and Python allow.
            (
                "fn main() {\n    print(\"{:>8}\")\n}\n",
                vec![(EmptyInterpolation, (2, 12))],
            ),
            (
                "fn main() {\n    let x = 1\n    let y = 2\n    print(\"{x:03} {y:+}\")\n}\n",
                vec![
                    (UnsupportedFormatSpec, (4, 14)),
                    (UnsupportedFormatSpec, (4, 21)),
                ],
            ),
            // An item that another error breaks reports its earliest error, a spec's included.
            (
                "fn main() {\n    let x = 1\n    print(\"{x:+}\")\n    let = 2\n}\n",
                vec![(UnsupportedFormatSpec, (3, 14))],
            ),
            (
                "fn main() {\n    let = 2\n    print(\"{2:+}\")\n}\n",
                vec![(Expected, (2, 9))],
            ),
            (
                "class A {\n    fn m(self) {\n        print(\"{1:+}\")\n    }\n    let n = 1\n}\nfn main() {}\n",
                vec![(UnsupportedFormatSpec, (3, 18))],
            ),
        ] {
            assert_eq!(codes(src), expected, "{src}");
        }
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
        // A broken constant poisons a call of its name too, whose arguments are still checked.
        let src = "const print = 1 2\nfn main() {\n    print(1, 2)\n    print(nope)\n}\n";
        assert_eq!(
            codes(src),
            [(Code::Expected, (1, 17)), (Code::BindingNotFound, (4, 11))]
        );
        let src = "const i64 = 1 2\nfn main() {\n    let n = i64(true, 1)\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (1, 15))]);
    }

    /// A constant runs to the end of its line, so text skipped after its value is its own: the
    /// constant is broken, and the items after it are still checked.
    #[test]
    fn skipped_text_after_a_constant_lies_in_it() {
        for (skipped, code) in [
            ("$", Code::UnexpectedCharacter),
            ("\"const", Code::LiteralNotClosed),
        ] {
            let src = format!(
                "const A = {{ 1 }} {skipped}\nconst B: u8 = 300\nfn main() {{\n    print(A)\n    let b: bool = 1\n}}\n"
            );
            assert_eq!(
                codes(&src),
                [
                    (code, (1, 17)),
                    (Code::IntegerDoesNotFit, (2, 15)),
                    (Code::Mismatch, (5, 19))
                ],
                "{src}"
            );
        }
    }

    /// A broken constant is declared, so its uses are poisoned; it poisons a function of the same
    /// name, and a broken function poisons a constant of the same name.
    #[test]
    fn broken_constant_poisons_its_uses() {
        for (src, at) in [
            (
                "const A = 1 2\nfn main() {\n    let b: bool = A\n    A = 2\n}\n",
                (1, 13),
            ),
            (
                "const A = 1 2\nfn A() {}\nfn main() {\n    A(1)\n}\n",
                (1, 13),
            ),
            (
                "fn A(x) {}\nconst A = 1\nfn main() {\n    let b: bool = A\n    A = 2\n}\n",
                (1, 7),
            ),
        ] {
            assert_eq!(codes(src), [(Code::Expected, at)], "{src}");
        }
        // A line that starts with `const` ends the unnamed item before it.
        let src = "def f()\nconst A = 1 2\nfn main() {}\n";
        assert_eq!(
            codes(src),
            [(Code::Expected, (1, 1)), (Code::Expected, (2, 13))]
        );
    }

    /// The fix of a `const` statement leaves a program that passes the whole front end.
    #[test]
    fn const_statement_fix_passes_the_front_end() {
        let src = "fn main() {\n    const n = 1\n    print(n)\n}\n";
        let ds = front_end(src).1;
        let [d] = ds.as_slice() else {
            panic!("expected one diagnostic, got {ds:?}");
        };
        assert_eq!(d.code(), Code::ConstInFunction);
        assert!(fix_edits(d).is_some_and(|(safe, _)| safe));
        let fixed = apply_fix(src, d);
        assert!(front_end(&fixed).1.is_empty(), "{fixed}");
    }

    /// The fix of a `}` in a string leaves a program that passes the whole front end, after an
    /// escaped backslash, after an interpolation and in a string inside one.
    #[test]
    fn brace_in_string_fix_passes_the_front_end() {
        for text in [r#""}""#, r#""\\}""#, r#""{n}}""#, r#""{ "}" }""#] {
            let src = format!("fn main() {{\n    let n = 1\n    print({text})\n}}\n");
            let ds = front_end(&src).1;
            let [d] = ds.as_slice() else {
                panic!("expected one diagnostic, got {ds:?}");
            };
            assert_eq!(d.code(), Code::BraceInString, "{src}");
            assert!(fix_edits(d).is_some_and(|(safe, _)| safe));
            let fixed = apply_fix(&src, d);
            assert!(front_end(&fixed).1.is_empty(), "{fixed}");
        }
    }

    /// A bracket left open in an interpolation that is not closed does not stop the lexer, so a
    /// later function is still checked.
    #[test]
    fn unclosed_interpolation_keeps_later_items() {
        let src = "fn main() {\n    let s = \"a{ f(x\n}\nfn g() {\n    let b: bool = 1\n}\n";
        assert_eq!(
            codes(src),
            [(Code::LiteralNotClosed, (2, 13)), (Code::Mismatch, (5, 19))]
        );
    }

    /// A literal cut at its line end leaves one token, whichever segment the cut ends: its
    /// first, one resumed after a `}`, or a literal nested in an interpolation. So the error in
    /// the literal is reported, its function ends at its `}`, and the next function is checked.
    #[test]
    fn cut_literal_leaves_one_token() {
        for (literal, escape) in [
            ("\"a\\q", 15),
            ("\"a{x}b\\q", 19),
            ("\"a{x}b{y}c\\q", 23),
            ("\"a{x}b\\q{y", 19),
            ("\"a{ \"b\\q", 19),
            ("\"a{ \"b{x}c\\q", 23),
        ] {
            let src = format!(
                "fn main() {{\n    let s = {literal}\n}}\nfn g() {{\n    let b: bool = 1\n}}\n"
            );
            assert_eq!(
                codes(&src),
                [
                    (Code::UnknownEscape, (2, escape)),
                    (Code::Mismatch, (5, 19))
                ],
                "{literal}"
            );
        }
    }

    #[test]
    fn unnamed_item_stops_the_checker() {
        let src = "def total() {}\nfn main() {\n    let b: bool = 1\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (1, 1))]);
    }

    /// A string that is not closed outside every item is an unnamed item, so the checker does
    /// not run: the string may have swallowed an item, and no name lookup can be trusted.
    #[test]
    fn unclosed_string_outside_items_stops_the_checker() {
        for (src, at) in [
            ("\"fn helper() {}\nfn main() {\n    helper()\n}\n", (1, 1)),
            (
                "\"class Helper {}\nfn helper() -> Helper {\n    return Helper()\n}\nfn main() {}\n",
                (1, 1),
            ),
            ("\"fn main() {}\n", (1, 1)),
            ("\"const A = 1\nfn main() {\n    print(A)\n}\n", (1, 1)),
            (
                "fn f() {} \"a note\nfn main() {\n    let b: bool = 1\n}\n",
                (1, 11),
            ),
        ] {
            assert_eq!(codes(src), [(Code::LiteralNotClosed, at)], "{src}");
        }
        // Skipped text keeps the checker on.
        let src = "fn f() {} $\nfn main() {\n    let b: bool = 1\n}\n";
        assert_eq!(
            codes(src),
            [
                (Code::UnexpectedCharacter, (1, 11)),
                (Code::Mismatch, (3, 19))
            ]
        );
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
        assert!(fix_edits(d).is_some_and(|(safe, _)| safe));
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
        let src = "export fn main() {\n    let = 1\n}\n";
        assert_eq!(codes(src), [(Code::Expected, (1, 1))]);
        assert_eq!(
            codes("fn main() {}\nstruct P {}\n"),
            [(Code::Expected, (2, 1))]
        );
    }

    #[test]
    fn lexer_error_after_a_nested_block() {
        let src = "fn f() {\n    if true {\n    }\n    let n: i64 = $\n}\nfn main() {}\n";
        assert_eq!(codes(src), [(Code::UnexpectedCharacter, (4, 18))]);
    }

    #[test]
    fn lexer_error_breaks_only_its_method() {
        let src = "class C {\n    n: i64\n    fn m(self) {\n        let n: i64 = $\n    }\n}\nfn main() {\n    let c = C(n: true)\n}\n";
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
