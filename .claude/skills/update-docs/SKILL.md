---
name: update-docs
description: Use in the Sisu repo before opening a pull request or after one merges, when a change to sisuc, the runtime, CLI flags, language features, the toolchain, the repository layout, or milestone status may have left the README, guides, architecture diagram, roadmap, glossary or ADRs stale.
---

# Update the docs after a change

The docs describe what `main` does. After each change, bring them back in
line with the code. Change only what the diff made untrue or incomplete,
check every claim against the code or a real run, and report what you
changed and what you left alone.

## 1. Find the change

Run plain git from the repository root, one command per call.

- Before a pull request: `git diff --stat origin/main...HEAD`, then
  `git diff origin/main...HEAD -- <path>` for each file that matters.
- After a merge: `git show --stat <merge-sha>`, then
  `git diff <merge-sha>^1 <merge-sha> -- <path>`.
- For a branch that is not checked out: `git diff origin/main...<branch>`,
  and `git show <branch>:<path>` to read a file as that branch has it.

If the pull request exists, read its body with `gh pr view <number>`. It
names what is out of scope and what does not work yet.

## 2. Map the change to the docs

Read each target below before you decide it needs no edit.

| The diff changes | Check and update |
| --- | --- |
| Flags or usage text in `crates/sisuc/src/main.rs` | The "Look inside the compiler" table in `docs/development.md`; the Driver card in `docs/architecture/sisu-pipeline.json` |
| A stage, or what passes between stages (`lexer.rs`, `parser.rs`, `ast.rs`, `check/`, `tir.rs`, `codegen.rs`, `link.rs`), including which stage lowers which sugar | The stage paragraphs in `docs/architecture.md`; the diagram (step 4) |
| Functions exported by `crates/runtime` | The Runtime paragraph in `docs/architecture.md`; the Runtime card in the diagram JSON |
| A language feature that did not compile on `main` before and now runs end to end (a new program in `crates/sisuc/tests/programs`) | "What works today" in `README.md`; `GLOSSARY.md` for any new term |
| A milestone starting or finishing | The milestone line in `README.md`; the milestone table in `docs/roadmap.md` |
| A decision the pull request states, with the alternatives it rejected | A new ADR in `docs/adr/`, numbered after the last one; "Decided in milestone N" in `docs/roadmap.md` |
| Behavior that a "Decided in milestone N" bullet in `docs/roadmap.md` describes | Rewrite the bullet to the current fact and name the milestone that changed it, for example "desugared by the checker (moved from the parser in milestone 2)" |
| Diagnostic or panic text | The sample outputs in `docs/development.md` |
| The toolchain, a prerequisite, a command, how tests run (`crates/sisuc/tests/*.rs`) or CI (`.github/workflows/`, `prek.toml`) | "Prerequisites" and "Build and test" in `docs/development.md`; the Commands table in `AGENTS.md`; step 5 of `CONTRIBUTING.md` |
| A top-level directory or crate | "Repository layout" in `docs/development.md`; the crate list in `AGENTS.md` |
| `crates/sisuc/tests/programs/fib.sisu` | The example in `README.md`, which must stay identical to it |

If no row matches, report "no doc changes needed" and why. Do not edit to
show activity.

Never write to `docs/superpowers/`. It holds internal specs and plans, is
gitignored, and the docs do not link to it.

## 3. Verify before you write

- Every command and output in the docs comes from a run on this commit.
  Build with `cargo build --workspace` in the checkout of that commit and run
  its `target/debug/sisuc`. A binary from another checkout may come from
  another branch. Never write output from memory or from the diff.
- Confirm each path, flag and function name in the code. Use CodeGraph when
  `.codegraph/` exists, otherwise grep.
- Write a path that is not on `main` yet as code, not as a link.
- Use the terms in `GLOSSARY.md` as spelled there, for example `tir` and
  poison type.
- A change that contradicts an ADR gets a new ADR that supersedes it. Do not
  edit the old ADR's decision.

## 4. Write

- If the `clarity` skill is available, use it in rewrite mode on each
  paragraph you add or rewrite.
- Match the surrounding text: short sentences, present tense, the same line
  width.
- Describe what the code does now. Status words such as "in progress" or
  "planned" belong only in the README milestone line and the roadmap.
- Keep the README short. A new capability is one bullet, not a section.
- The diagram: edit `docs/architecture/sisu-pipeline.json`, then regenerate
  both SVGs as "Updating the diagram" in `docs/architecture.md` describes.
  Never edit the SVGs by hand. If you cannot run archify, update the JSON
  and report that the SVGs are stale.

## 5. Check

```console
$ python3 .claude/skills/update-docs/scripts/check_links.py README.md CONTRIBUTING.md AGENTS.md docs/*.md
broken: 0
$ awk '/^```/{f=!f;next} f' README.md | diff - crates/sisuc/tests/programs/fib.sisu
$ prek run --files <each changed file>
```

The `awk` line prints nothing when the README example matches `fib.sisu`.

## 6. Report and commit

Report, in this order:

1. Each file you changed, with one line on why.
2. How you verified each new claim: the command you ran, or the code you read.
3. Each row from step 2 you checked and left alone, with the reason.
4. Anything still stale, such as SVGs you could not regenerate.

Before a pull request, commit the doc changes on the same branch. After a
merge, put them on a new `docs/<topic>` branch and open a pull request.
Never merge it; the user reviews and merges.
