# Sisu

[![CI](https://github.com/mikeleppane/sisu/actions/workflows/ci.yml/badge.svg)](https://github.com/mikeleppane/sisu/actions/workflows/ci.yml)

A small, statically typed language that compiles to native x86-64 Linux code
through LLVM.

Sisu (Finnish for grit) is a learning project: the goal is to build a language
end to end, from source text to a running binary, not to ship a production
language. The compiler, `sisuc`, is written in Rust and drives LLVM 18 through
[inkwell](https://github.com/TheDan64/inkwell).

```
fn fib(n: i64) -> i64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

fn main() {
    print_int(fib(30))
}
```

## Status

Early work: the workspace and tooling are in place, the compiler is not yet.

| # | Milestone | Done when |
| --- | --- | --- |
| 1 | Number crunching: `i64`, `bool`, functions, `if`, `while`, recursion | Fibonacci and a prime counter print correct answers |
| 2 | Classes, methods, heap, optionals, reference counting | A linked list and a binary tree run clean under Valgrind |
| 3 | Arrays and a byte type | A prime sieve prints correct answers |
| 4 | Strings | A program reverses and compares strings |
| 5 | Input and output | A word-count tool reads standard input |
| 6 | Floats | A Mandelbrot set prints as text |

## Development

You need the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs it
on first use) and LLVM 18 (`llvm-18-dev` on Ubuntu).

```console
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
