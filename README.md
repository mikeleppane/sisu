# Sisu

[![CI](https://github.com/mikeleppane/sisu/actions/workflows/ci.yml/badge.svg)](https://github.com/mikeleppane/sisu/actions/workflows/ci.yml)

A small, statically typed language that compiles to native x86-64 Linux code
through LLVM.

Sisu (Finnish for grit) is a learning project: the goal is to build a language
end to end, from source text to a running binary, not to ship a production
language. The compiler, `sisuc`, is written in Rust and drives LLVM 22 through
[inkwell](https://github.com/TheDan64/inkwell).

```
fn fib(n: i64) -> i64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

fn main() {
    print(fib(30))
}
```

## Status

Milestone 1 is done: `sisuc` compiles `i64`, `bool`, functions, `if`, `while`
and recursion to a native executable.

| # | Milestone | Done when | Status |
| --- | --- | --- | --- |
| 1 | Number crunching: `i64`, `bool`, functions, `if`, `while`, recursion | Fibonacci and a prime counter print correct answers | Done |
| 2 | Classes, methods, heap, reference counting, optionals with `?.` and `??`, `break` and `continue` | A linked list and a binary tree run clean under Valgrind | Planned |
| 3 | Arrays and a byte type | A prime sieve prints correct answers | Planned |
| 4 | Strings | A program reverses and compares strings | Planned |
| 5 | Input and output | A word-count tool reads standard input | Planned |
| 6 | Floats | A Mandelbrot set prints as text | Planned |

## Development

You need the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs it
on first use), and `libzstd-dev`, `libxml2-dev` and `zlib1g-dev` on Ubuntu.
LLVM 22 lives inside the repo: `scripts/install-llvm.sh` downloads the official
release (1.9 GB) and keeps only what the build needs in `.llvm/22` (about
360 MB). `.cargo/config.toml` points the build at it. Any system LLVM is left
alone.

```console
$ ./scripts/install-llvm.sh  # once per clone: LLVM 22 into .llvm/
$ cargo build --workspace
$ cargo nextest run --workspace
$ prek install            # once per clone: run the checks on every commit
$ prek run --all-files    # fmt, clippy, typos, file hygiene, as in CI
$ cargo deny check        # advisories, licenses, sources
```

## Repository layout

| Path | What it is |
| --- | --- |
| `crates/sisuc` | The compiler: lexer, parser, type checker, LLVM IR generation, linking. |
| `crates/runtime` | `sisu-runtime`, a static library with a C ABI linked into every compiled program. |
| `docs` | Project and agent documentation. |
