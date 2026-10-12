# Developing Sisu

This guide covers building `sisuc`, compiling a Sisu program with it, and
running the same checks as CI. Sisu builds and runs on x86-64 Linux.

## Prerequisites

- The Rust toolchain pinned in `rust-toolchain.toml`. rustup installs it on
  first use.
- A C compiler on `PATH` as `cc`. `sisuc` uses it to link programs.
- On Ubuntu: `libzstd-dev`, `libxml2-dev` and `zlib1g-dev`, which LLVM links
  against.
- [prek](https://prek.j178.dev) for the Git hooks, with
  [typos](https://github.com/crate-ci/typos) and `shellcheck`, which two of
  the hooks run.
- [cargo-nextest](https://nexte.st). Run the tests with it, not
  `cargo test`: its setup script builds the runtime library that the
  end-to-end tests link against.
- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) and
  [cargo-mutants](https://mutants.rs) for the commands below.
- Valgrind. The end-to-end tests run each program under it to catch leaks
  and memory errors.

LLVM 22 lives inside the repository. `scripts/install-llvm.sh` downloads the
official release (1.9 GB) and keeps only what the build needs in `.llvm/22`
(about 360 MB). `.cargo/config.toml` points the build at it, and any system
LLVM is left alone. [Upgrading LLVM](llvm.md) covers moving to a new major.

## Set up a clone

Run these once per clone:

```console
$ ./scripts/install-llvm.sh             # LLVM 22 into .llvm/22
$ prek install                          # run the checks on every commit
$ cargo install --locked cargo-mutants  # mutation testing
```

## Build and test

```console
$ cargo build --workspace        # sisuc and the runtime library
$ cargo nextest run --workspace  # unit tests and end-to-end cases
$ prek run --all-files           # fmt, clippy, expect messages, typos, file hygiene, shell and workflow lint, as in CI
$ cargo deny check               # advisories, licenses, sources
```

The end-to-end cases are files. The `cases` test binary runs one test per
`.sisu` file in three directories under `crates/sisuc/tests`:

| Directory | The program must | Expected output |
| --- | --- | --- |
| `run/` | exit 0 with empty stderr, in a plain and an `-O2` build, both also under Valgrind, and in a `--rc-checks` build | `<name>.out`: stdout |
| `run-fail/` | panic with exit code 101, the same in a plain and an `-O2` build | `<name>.out`: stdout; `<name>.err`: stderr |
| `compile-fail/` | fail `sisuc --check` with exit code 1 | `<name>.err`: the diagnostics |

To add a case, write the `.sisu` file and an empty expected file beside it,
then fill the expected file with `SISU_BLESS=1`. Read what it wrote before
you commit it:

```console
$ touch crates/sisuc/tests/run/new_case.out
$ SISU_BLESS=1 cargo nextest run -E 'binary(cases)'
```

`SISU_BLESS=1` rewrites the expected files of cases that pass every other
check, and only those. It never creates a file. Any other file in these
directories fails the run: a `.out` or `.err` without its `.sisu`, an
expected file that the directory's mode does not read, or a hidden file.

A `run/` case whose output depends on a value crossing a real call says so
with a marker line, `// o2-calls: <caller> <callee> <count>`. The test then
checks that the `-O2` IR of `<caller>` holds exactly `<count>` calls to
`<callee>`, because the inliner can fold a call away and keep the output
the same. Every function except the C `main` is internal, so `-O2` would
inline a caller with one call site and the caller would vanish from the IR.
The check therefore runs `sisuc -O2 --emit ir` once for each distinct
`<caller>`, with the environment variable `SISU_KEEP_FUNCTIONS` set to that
caller alone. That marks it `noinline` in the module and changes nothing
else; `sisuc` reads it only for `--emit ir`, and the usage text does not
mention it. One caller per build, so that keeping one caller does not also
keep a callee that another marker names as its caller. The check counts the
calls to `<callee>` in the body of `<caller>` after `-O2`. It fails with "no
function named" if `<caller>` or `<callee>` does not exist, and with
"takes parameters" if `<caller>` has any: kept as a function of its own, such
a caller could hold a call that its real call sites decide away, so mark a
caller without parameters, such as `main`. The check measures the marked caller
as a standalone function. In a real build the caller is inlined, and a call to a
callee that LLVM infers as `memory(none)` or `readonly` can then be merged with an
identical call at the call site. So mark calls to callees that write memory, as
every current marker does. The old external-linkage check measured the standalone
caller the same way. Interprocedural constant propagation also runs on internal
functions, so a callee that keeps its calls recurses to a depth that the call sites
do not fix; see `handover.sisu`.
`crates/sisuc/tests/programs` holds the few programs that the small-stack
tests in `programs.rs` run.

Mutation testing changes the code that a branch touched, one mutant at a
time, and reports each mutant the tests still pass. The script mutates the
diff from where the branch left `main`. On a branch stacked on another stage,
pass that stage's tip as the base, so earlier stages stay out. Arguments after
the base go to `cargo mutants`:

```console
$ scripts/mutants.sh                  # changes since main
$ scripts/mutants.sh feat/m3-writes   # changes since the stage below
$ scripts/mutants.sh HEAD~1 --list    # list the mutants, run nothing
```

The diff includes uncommitted edits. A new file counts once `git add` tracks
it.

## Compile and run a program

`sisuc` takes a source file and the name of the executable to write:

```console
$ cargo build --workspace
$ target/debug/sisuc crates/sisuc/tests/run/fib.sisu fib
$ ./fib
832040
```

`sisuc` links every program with `libsisu_runtime.a`, which it looks for
next to its own executable. `cargo build --workspace` puts both files there;
`cargo build -p sisuc` does not build the runtime.

When a program breaks a rule, `sisuc` prints a diagnostic to stderr and exits
with code 1. Take this `bad.sisu`:

```
fn main() {
    let x: bool = 1
}
```

```console
$ target/debug/sisuc bad.sisu bad
error: expected `bool`, found `i64`
 --> bad.sisu:2:19
  |
2 |     let x: bool = 1
  |                   ^
```

When a compiled program fails at runtime, for example by dividing by zero,
it prints a panic to stderr and exits with code 101:

```console
$ target/debug/sisuc crates/sisuc/tests/run-fail/div_zero.sisu div_zero
$ ./div_zero
sisu: panic at crates/sisuc/tests/run-fail/div_zero.sisu:2:5: division by zero
```

`print` writes into a buffer that the runtime owns, as C stdio does: a line
at a time when standard output is a terminal, in blocks of 8 KiB otherwise.
The runtime flushes the buffer when `main` returns, before every Sisu panic
message, and before the `--rc-checks` exit message, so a panic's stdout lines
come out before its stderr line. A program that an outside signal kills, such
as a stack overflow, loses what is still in the buffer.

A `print` or the final flush that cannot write to standard output is a panic
too, for example when the disk is full:

```console
$ ./program >/dev/full
sisu: panic: standard output could not be written: No space left on device (os error 28)
```

A pipe with no reader is the exception, as in C: writing to it kills the
program with SIGPIPE. A panic ignores SIGPIPE on its way out, so it exits with
code 101 even when nothing reads standard error.

## Look inside the compiler

These flags run the pipeline up to one stage and stop there.
[Architecture](architecture.md) describes the stages.

| Command | Prints |
| --- | --- |
| `sisuc --emit tokens file.sisu` | The lexer's tokens |
| `sisuc --emit ast file.sisu` | The parser's syntax tree |
| `sisuc --emit tir file.sisu` | The checker's typed tree, `tir` |
| `sisuc --emit ir file.sisu` | The LLVM IR, before and after `mem2reg` |
| `sisuc -O2 --emit ir file.sisu` | The LLVM IR, before and after LLVM's `default<O2>` pipeline |
| `sisuc --emit ir-raw file.sisu` | The LLVM IR before any pass, as one module that LLVM's `opt` reads |
| `sisuc --check file.sisu` | Only diagnostics; exits with code 1 if any is an error |

`-O2` must come first. It also works when building an executable:
`sisuc -O2 file.sisu out`.

`sisuc --rc-checks file.sisu out` builds an executable that checks its
reference counts. Before each retain or release, it checks that the count is
at least 1. At a normal exit, it checks that no object is still allocated.
A failed check prints a line that starts with `sisu: rc check:` and exits
with code 102. `--rc-checks` comes after `-O2`, if both are given, and works
only when building an executable. A use after free often goes unseen: the
freed memory may still hold a count of 1 or more. Valgrind finds those.

## Repository layout

| Path | What it is |
| --- | --- |
| `crates/sisuc` | The compiler: lexer, parser, checker, codegen and linking. |
| `crates/sisuc/tests/run`, `run-fail`, `compile-fail` | End-to-end cases, each with the output it must print. |
| `crates/sisuc/tests/programs` | The programs the small-stack tests run. |
| `crates/runtime` | `sisu-runtime`, a static library with a C ABI linked into every compiled program. |
| `scripts` | `install-llvm.sh`, `mutants.sh`, and `check-expect-messages.sh`, which a prek hook runs. |
| `docs` | Architecture, roadmap, decisions (`adr/`) and agent guides (`agents/`). |
