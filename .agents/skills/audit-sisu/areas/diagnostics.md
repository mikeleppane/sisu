# Area: diagnostics

Every program the language forbids gets a diagnostic that points at the
code at fault and says what is wrong in the user's terms. The rules for
messages are in `CODING_STANDARDS.md`, "Diagnostics"; the existing cases
are in `crates/sisuc/tests/compile-fail` and the unit tests of the lexer,
parser and checker.

## Probes

- For each kind of error the lexer, parser and checker report, write the error in three new places (a nested expression, a method body, a loop) and check that the span underlines the right code, on the right line and column, including after a multibyte character and on the last line with no trailing newline.
- Untested messages: list the messages the `Diagnostic` constructors in `crates/sisuc/src` build. A message that neither a `compile-fail/*.err` file nor a unit test asserts is a `gap`.
- Cascades: an expression that already failed gets the poison type, so it must not produce a second diagnostic further on. Give a function the wrong return type and use its result in five expressions; misspell a field and use the result. A diagnostic caused only by an earlier one is a finding. Separate uses of an unknown name may each get their own.
- Recovery: the lexer and parser stop at the first error; the checker reports every error. Two independent checker errors in one file must both appear.
- Wording: a message that uses compiler terms (`tir`, poison, internal type names), suggests a fix that would not compile, or names the wrong type.

Severity is `gap`, unless a forbidden program gets no diagnostic at all
(then `wrong`) or `sisuc` fails (then `crash`).
