# Coding standards

Reviewers apply these rules to every diff. Clippy, rustfmt and prek already enforce the mechanical rules (`[workspace.lints]` in `Cargo.toml`). This file holds the judgement calls that no check can make. Report a broken rule as a finding, with file:line.

## Tests

- **A test proves its name.** It must fail when the contract it names breaks. For example, a test that byte offsets are not character offsets needs non-ASCII input. A test that a line break inside `( )` inserts no `Newline` needs the break after a token that could end a statement.
- **The spec's messages have tests.** Every diagnostic message the spec names has a test that asserts its text and its `line:col`. Every layout rule in "Statements end at newlines" has a case outside `( )` and a case inside.
- **IR tests check the contract, not LLVM's naming.** Match opcodes, predicates, intrinsic names and block-name prefixes. Value names such as `%0` or `%cmp` change when unrelated code changes. Prove runtime behaviour end to end in `crates/sisuc/tests/programs`.
- **Plan-pinned tests keep their expected values.** When behaviour grows, add a new case.
- **Every surviving mutant is a finding.** Run `cargo mutants` on the diff (see `AGENTS.md`). Each survivor needs a test that kills it, or a note in the report on why it is equivalent.
- **A call boundary is proven in `-O2` IR.** A test that relies on a value crossing a real call shows the `call` in the `-O2` output. Matching program output is not enough, because the inliner can fold the call away.
- **An evaluation-order test fails on any swap.** Give each operand its own visible effect, such as a distinct `print` or a distinct panic, so that running any two in the other order changes the output. `cargo mutants` does not reorder operands, so no survivor flags a test that misses this.
- **Programs that exit 0 run three ways.** Every such program in `crates/sisuc/tests/programs` goes through `assert_prints_out_file`: plain, under Valgrind, and at `-O2` under Valgrind. `every_program_has_a_run` checks that it is listed. Check each `.out` file by hand against the program, and comment each printed line with its value.
- **Complexity is tested by timeout, not by timing.** Use an input large enough to hang on a quadratic path, and give the test a nextest `terminate-after` override in `.config/nextest.toml`. Never assert on wall-clock time.

## Staging work across tasks

- **No reachable crash lands in a stage.** A feature that a later task implements is rejected by the checker until then, never left to a codegen panic. The diagnostic names the task that removes it.
- **A stage's later arms panic by name.** Inside one stage, when a task adds a syntax or `tir` node that a later task of the same stage handles, the later code's match arm is `panic!("<node> lands in Task N")`, and Task N removes it. The stage's last task leaves none behind.
- **A restriction on a type sits where the type is made.** Put it in the constructor every path goes through (as `optional_of` does), not where an annotation is read.

## Code

- **An `expect` message states the invariant.** Write what should hold and why, with "should", naming the thing involved: `the checker should reject \`break\` outside a loop`. The panic then reads as a broken assumption. "builder is positioned" describes success, and the error that follows it reads as noise. A prek hook checks that non-test messages say "should"; the reviewer checks that they name the invariant and why it holds.

- **Sizes come from `TargetData`.** Sizes and alignments in generated code are measured with LLVM's `TargetData`, never written as numbers. The 16-byte array header is two `i64`s measured the same way.

## Diagnostics

- **One rule, one message.** A rule reports the same wording wherever it fires: lexer or parser, inside or outside brackets.
- **The primary span covers the offending code.** For "end of line" and "end of file", it is an empty span at that point. `Diagnostic::render` asserts in debug builds that every span sits on character boundaries and does not end with a line break.

## Files the compiler writes

- **Leave the input and existing files intact.** An output path that names the input is an error. Intermediate files go to fresh, exclusively created, owner-only paths, and are removed on every exit path.
