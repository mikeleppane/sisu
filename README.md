<p align="center">
  <img src="docs/images/sisu-logo.png" alt="Sisu logo: a steel letter S with glowing orange seams" width="160">
</p>

# Sisu

[![CI](https://github.com/mikeleppane/sisu/actions/workflows/ci.yml/badge.svg)](https://github.com/mikeleppane/sisu/actions/workflows/ci.yml)

A small, statically typed language that compiles to native x86-64 Linux code
through LLVM.

Sisu (Finnish for grit) is a learning project: the goal is to build a language
end to end, from source text to a running binary, not to ship a production
language. The compiler, `sisuc`, is written in Rust and drives LLVM 22 through
[inkwell](https://github.com/TheDan64/inkwell).

This program prints the 30th Fibonacci number, `832040`:

```
fn fib(n: i64) -> i64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

fn main() {
    print(fib(30))
}
```

## What works today

Milestones 1 and 2 are done. `sisuc` compiles:

- `i64` and `bool`, with `let` for immutable bindings, `var` for variables,
  and inferred types
- functions and recursion
- `if` as an expression, `while`, `break`, `continue` and `return`
- comparison chains such as `0 <= i < n`, and short-circuit `&&` and `||`
- integer arithmetic that panics on overflow and division by zero instead of
  wrapping
- classes with fields and methods, whose objects are freed by reference
  counting when the last reference goes away
- `==` that compares objects field by field, and `is` that tests identity
- optionals such as `i64?` and `Node?`, with `None`, `?.`, `??`, `if let` and
  `while let`
- error messages in the style of `rustc`, pointing at the code

## Start here

| You want to | Read |
| --- | --- |
| Build Sisu and compile a program | [Developing Sisu](docs/development.md) |
| See how the compiler is put together | [Architecture](docs/architecture.md) |
| See what is planned | [Roadmap](docs/roadmap.md) |
| Change the code | [Contributing](CONTRIBUTING.md) |
| Look up a term | [Glossary](GLOSSARY.md) |
| Read why a design choice was made | [Decisions](docs/adr) |
