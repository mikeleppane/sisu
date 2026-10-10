# Coding standards

Reviewers apply these rules to every diff. Clippy, rustfmt and prek already enforce the mechanical rules (`[workspace.lints]` in `Cargo.toml`). This file holds the judgement calls that no check can make. Report a broken rule as a finding, with file:line.

## Tests

- **A test proves its name.** It must fail when the contract it names breaks. For example, a test that byte offsets are not character offsets needs non-ASCII input. A test that a line break inside `( )` inserts no `Newline` needs the break after a token that could end a statement.
- **The spec's messages have tests.** Every diagnostic message the spec names has a test that asserts its text and its `line:col`. Every layout rule in "Statements end at newlines" has a case outside `( )` and a case inside.
- **IR tests check the contract, not LLVM's naming.** Match opcodes, predicates, intrinsic names and block-name prefixes. Value names such as `%0` or `%cmp` change when unrelated code changes. Prove runtime behaviour end to end in `crates/sisuc/tests/programs`.
- **Plan-pinned tests keep their expected values.** When behaviour grows, add a new case.
- **Every surviving mutant is a finding.** Run `cargo mutants` on the diff (see `AGENTS.md`). Each survivor needs a test that kills it, or a note in the report on why it is equivalent.
- **A call boundary is proven in `-O2` IR.** A test that relies on a value crossing a real call shows the `call` in the `-O2` output. Matching program output is not enough, because the inliner can fold the call away.
- **Complexity is tested by timeout, not by timing.** Use an input large enough to hang on a quadratic path, and give the test a nextest `terminate-after` override in `.config/nextest.toml`. Never assert on wall-clock time.

## Staging work across tasks

- **No reachable crash lands in a stage.** A feature that a later task implements is rejected by the checker until then, never left to a codegen panic. The diagnostic names the task that removes it.
- **A restriction on a type sits where the type is made.** Put it in the constructor every path goes through (as `optional_of` does), not where an annotation is read.

## Diagnostics

- **One rule, one message.** A rule reports the same wording wherever it fires: lexer or parser, inside or outside brackets.
- **The primary span covers the offending code.** For "end of line" and "end of file", it is an empty span at that point. `Diagnostic::render` asserts in debug builds that every span sits on character boundaries and does not end with a line break.

## Files the compiler writes

- **Leave the input and existing files intact.** An output path that names the input is an error. Intermediate files go to fresh, exclusively created, owner-only paths, and are removed on every exit path.
