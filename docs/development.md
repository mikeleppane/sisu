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
- [cargo-nextest](https://nexte.st) and
  [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) for the commands
  below.

LLVM 22 lives inside the repository. `scripts/install-llvm.sh` downloads the
official release (1.9 GB) and keeps only what the build needs in `.llvm/22`
(about 360 MB). `.cargo/config.toml` points the build at it, and any system
LLVM is left alone.

## Set up a clone

Run these once per clone:

```console
$ ./scripts/install-llvm.sh   # LLVM 22 into .llvm/22
$ prek install                # run the checks on every commit
```

## Build and test

```console
$ cargo build --workspace        # sisuc and the runtime library
$ cargo nextest run --workspace  # unit tests and end-to-end programs
$ prek run --all-files           # fmt, clippy, typos, file hygiene, shell and workflow lint, as in CI
$ cargo deny check               # advisories, licenses, sources
```

## Compile and run a program

`sisuc` takes a source file and the name of the executable to write:

```console
$ cargo build --workspace
$ target/debug/sisuc crates/sisuc/tests/programs/fib.sisu fib
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
$ target/debug/sisuc crates/sisuc/tests/programs/div_zero.sisu div_zero
$ ./div_zero
sisu: panic at crates/sisuc/tests/programs/div_zero.sisu:2:5: division by zero
```

## Look inside the compiler

These flags run the pipeline up to one stage and stop there.
[Architecture](architecture.md) describes the stages.

| Command | Prints |
| --- | --- |
| `sisuc --emit tokens file.sisu` | The lexer's tokens |
| `sisuc --emit ast file.sisu` | The parser's syntax tree |
| `sisuc --emit ir file.sisu` | The LLVM IR, before and after `mem2reg` |
| `sisuc --check file.sisu` | Only diagnostics; exits with code 1 if any is an error |

## Repository layout

| Path | What it is |
| --- | --- |
| `crates/sisuc` | The compiler: lexer, parser, checker, codegen and linking. |
| `crates/sisuc/tests/programs` | End-to-end programs, each with the output it must print. |
| `crates/runtime` | `sisu-runtime`, a static library with a C ABI linked into every compiled program. |
| `scripts` | `install-llvm.sh`. |
| `docs` | Architecture, roadmap, decisions (`adr/`) and agent guides (`agents/`). |
