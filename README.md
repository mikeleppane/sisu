<p align="center">
  <img src="docs/images/sisu-logo.png" alt="Sisu logo: a steel letter S with glowing orange seams" width="160">
</p>

# Sisu

[![CI](https://github.com/mikeleppane/sisu/actions/workflows/ci.yml/badge.svg)](https://github.com/mikeleppane/sisu/actions/workflows/ci.yml)

Sisu is a small, statically typed programming language that compiles to native
x86-64 Linux code through LLVM. Sisu is Finnish for grit.

Sisu is a passion project: an attempt to build a language that reads like
Python, checks like Rust and manages memory like Swift, and to build all of
it, from source text to a running binary. The compiler, `sisuc`, is written in
Rust and drives LLVM 22 through [inkwell](https://github.com/TheDan64/inkwell).

## Status

Sisu is very much under development. It has no releases, and any milestone may
break programs that compiled before it. It runs on x86-64 Linux only.

Milestones 1 to 3 are done: integers and booleans, classes and optionals, and
arrays. Strings are next. The [roadmap](docs/roadmap.md) has the rest.

## What Sisu values

The goal is that people are productive in Sisu because things just work. Four
values serve that goal, and when they conflict, this order settles it
([ADR 0006](docs/adr/0006-values-correctness-simplicity-readability-performance.md)):

1. **Correctness.** A program never gives a silent wrong answer. Failures are
   loud and name the source line. Nothing changes behind the reader's back: no
   hidden aliasing, no implicit conversions.
2. **Simplicity.** Few concepts, rules without special cases, one obvious way
   to do a thing, and every effect visible where it happens.
3. **Readability.** Little punctuation, code that reads top to bottom, and
   errors in plain words.
4. **Performance.** Predictable and good enough, in the range of Swift and
   Kotlin, and only from optimizations that do not change what a program
   means.

Simplicity ranks above readability because agents now write and read much of
the code. Special cases and effects that happen elsewhere cause their mistakes,
and a strict checker with plain-word errors lets them fix their own code.

## A taste of Sisu

Classes hold fields and methods. A `Tree?` is either a tree or `None`; `?.`
calls through it and `??` gives a fallback. Reference counting frees an object
when its last reference goes away.

<!-- GitHub has no Sisu grammar; Rust's is the closest, so the examples use it. -->

```rust
class Tree {
    value: i64
    left: Tree?
    right: Tree?

    fn sum(self) -> i64 {
        self.value + (self.left?.sum() ?? 0) + (self.right?.sum() ?? 0)
    }
}

fn leaf(value: i64) -> Tree {
    Tree(value: value, left: None, right: None)
}

fn main() {
    let tree = Tree(value: 1, left: leaf(2), right: leaf(3))
    print(tree.sum())               // 6
    print(tree.left?.value ?? 0)    // 2
}
```

Arrays are values. Assigning one shares it until one side is written, so
writing a copy never changes the original. `let mut` marks everything that can change,
and comparisons chain as in mathematics.

```rust
fn main() {
    let primes = [2, 3, 5, 7]
    let mut copy = primes    // shared until one of them is written
    copy[0] = 11
    print(primes.sum())      // 17
    print(copy.sum())        // 26
    for p in primes {
        if 2 < p <= 5 { print(p) }    // 3, then 5
    }
}
```

Integer arithmetic panics on overflow and on division by zero instead of
wrapping, and the panic names the line:

```rust
fn main() {
    let big = 9_223_372_036_854_775_807
    print(1)
    print(big + 1)
}
```

```console
$ sisuc overflow.sisu overflow && ./overflow
1
sisu: panic at overflow.sisu:4:11: integer overflow
```

## Errors that help

The checker reports every error in one run. Each diagnostic has a stable code,
points at the code, and offers a fix when it can:

```rust
class Counter {
    count: i64
}

fn main() {
    let total = 0
    total = 5
    let c = Countre(count: total)
    print(c.count)
}
```

```console
$ sisuc --check counter.sisu
error[E0001]: cannot assign to `total`
 --> counter.sisu:7:5
  |
6 |     let total = 0
  |         ----- declared with `let` here
7 |     total = 5
  |     ^^^^^^^^^ cannot assign twice
  |
help: declare it with `let mut`
  |
6 |     let mut total = 0
  |         +++

error[E0052]: cannot find function `Countre`
 --> counter.sisu:8:13
  |
8 |     let c = Countre(count: total)
  |             ^^^^^^^
  |
help: a class with a similar name exists: `Counter`
  |
8 -     let c = Countre(count: total)
8 +     let c = Counter(count: total)
  |

sisuc: 2 errors
sisuc: for more on a code, run `sisuc --explain <code>`: E0001, E0052
```

`sisuc --explain E0001` prints why the rule exists, with a broken and a fixed
program. `sisuc --format json` writes the same diagnostics as JSON Lines for
editors and agents, and marks each fix as certain or a guess.
[Diagnostic codes](docs/diagnostics/README.md) lists every code.

## Start here

| You want to | Read |
| --- | --- |
| Build Sisu and compile a program | [Developing Sisu](docs/development.md) |
| See how the compiler is put together | [Architecture](docs/architecture.md) |
| See what is planned | [Roadmap](docs/roadmap.md) |
| Compare Sisu's speed, memory and binary size with other languages | [Benchmarks](docs/benchmarks.md) |
| Change the code | [Contributing](CONTRIBUTING.md) |
| Look up a term | [Glossary](GLOSSARY.md) |
| Read why a design choice was made | [Decisions](docs/adr) |
