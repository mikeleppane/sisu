# Roadmap beyond milestone 6

Status: proposal, 2026-10-09. Nothing here is decided until it lands in a
milestone spec or an ADR.

The README lists milestones 1 to 6. They give Sisu a working imperative core:
`i64`, `bool`, functions, classes, reference counting, arrays, strings, I/O and
floats. This document collects what comes after, and the ergonomic features
that should be folded into milestones 2 to 6 while they are still cheap.

## Guiding goal: ergonomic and readable

Sisu should be pleasant to write and easy to read. The roadmap is ordered by
what makes everyday code nicer, not by compiler-theory depth. Features that
only help the compiler (debug info, an optimizing IR, concurrency) wait until
the language itself reads well.

Principles, recorded with the language mix in ADR 0004 so every later syntax
choice is checked against them:

- Code reads top to bottom, without punctuation noise.
- One obvious way to do a thing.
- Errors speak in plain words and point at the code.
- Memory is managed for you (reference counting, chosen in milestone 2).

Sisu takes its syntax's readability from Python, its correctness from Rust,
and its memory model and everyday ergonomics from Swift and Kotlin (ADR 0004).

### Target sample

Every later milestone should move real code toward this:

```
enum Shape {
    Circle(radius: f64)
    Rect(w: f64, h: f64)
}

fn area(s: Shape) -> f64 {
    match s {
        Circle(r)  => 3.14159 * r * r
        Rect(w, h) => w * h
    }
}

fn main() {
    let shapes = [Circle(1.0), Rect(2.0, 3.0)]
    var total = 0.0

    for s in shapes {
        total += area(s)
    }

    print("Total area: {total}")

    let name = env("USER") ?? "stranger"
    print("Hello, {name}!")
}
```

## Already adopted in milestone 1

These came out of the review of the milestone 1 spec and are now part of it:

- `let` immutable by default, `var` mutable (ADR 0002), with local type
  inference.
- `if` is an expression.
- No semicolons; statements end at newlines (ADR 0001).
- One built-in `print(e)` for `i64` and `bool` instead of `print_int`.
- Compound assignment `+= -= *= /= %=`, desugared by the parser.
- Digit separators: `100_000`.
- Diagnostics in the style of rustc: the code underlined, secondary labels
  such as "declared with `let` here", and a `help:` line.
- `break` and `continue` scheduled for milestone 2.
- From milestone 2, a line starting with `.` continues the previous line, so
  method chains can be written one call per line (ADR 0001).

## Fold into milestones 2 to 6

Each item is cheap when its milestone lands and expensive to retrofit:

| Milestone | Add | Why |
| --- | --- | --- |
| 2 | `?.` and `??` on optionals | No nested null checks |
| 2 | The checker reports every error in one run, using a poison type so one mistake does not cascade | One compile shows every type error |
| 2 | `==` on classes | Comparison without hand-written methods |
| 3 | Array literals `[1, 2, 3]` | Data reads like data |
| 3 | `for x in xs` and ranges `0..n` | Most loops stop needing indexes |
| 4 | String interpolation `"x = {x}"` | The biggest single win for real programs |

## Proposed milestones 7 to 12

| # | Milestone | Done when |
| --- | --- | --- |
| 7 | Enums with payloads, `match` with destructuring and an exhaustiveness check, `if let` | The shape sample above runs; a non-exhaustive `match` is a compile error |
| 8 | Generics (monomorphization) and interfaces, with `print` generic through a `Display`-like interface | `print(x)` works for any printable type; `T?` is shorthand for a library `Option<T>` |
| 9 | Closures and iterator methods (`map`, `filter`, `count`, `sum`) | `words.filter(w => w.len() > 3).count()` runs |
| 10 | Records, named arguments, default parameters | A `Point` record prints and compares with no hand-written methods |
| 11 | Modules, `pub` visibility, and a standard library written in Sisu (`List`, `Map`, string builder) | A word-frequency tool reads like a Python script |
| 12 | Error handling: `Result<T, E>` and a `?` operator | File I/O code has no nested error checks |

Order matters in two places:

- Milestone 7 comes before generics, because `Option` and `Result` are
  enums. Optionals still change twice: milestone 2 builds `T?` into the
  compiler, and milestone 8 turns it into shorthand for the library
  `Option<T>` (as in Swift). The syntax stays, so no program breaks.
- Milestone 8 comes before the standard library, which needs generic
  collections.
- Milestone 5 I/O comes before `Result`, so its functions report failure
  with an optional or a panic. Milestone 12 changes those signatures.

## Later and stretch goals

These do not make code nicer to write, so they wait:

- Weak references or a cycle collector. Reference counting leaks cycles, so a
  doubly linked list or a parent-pointer tree leaks without one. Decide before
  milestone 11 grows data structures that create cycles.
- C FFI: `extern fn` to call libc or a C library.
- A test runner: `test` blocks and `sisuc test`.
- Parser error recovery, so a syntax error does not hide the next one, and
  more warnings, such as unused bindings.
- Debug info (DWARF), so `gdb` steps through Sisu source.
- A typed mid-level IR with reference-count elision and other optimizations.
- A formatter (`sisuc fmt`) and a language server.
- Concurrency: threads, atomic reference counts, channels.
- Self-hosting: the Sisu lexer and parser written in Sisu.

## Deliberately avoided

- Lifetimes and a borrow checker. Reference counting keeps Sisu in the Swift
  and Kotlin ergonomic camp.
- Header files.
- Unrestricted operator overloading.

## Open decisions

- **How the checker hands types to codegen.** Milestone 1 codegen reads types
  from LLVM values, which works only for `i64` and `bool`. LLVM 22 pointers
  are opaque, so from milestone 2 a `ptr` cannot say which class it points
  to; the checker must record the types. A side table is enough at first; a
  typed, desugared tree is likely needed once `match`, closures and
  monomorphization land.
- **Generic arguments and `<`.** `f(a < b, c > d)` reads as two comparisons
  or as one generic call. Decide the syntax in milestone 8.
- **Weak references or a cycle collector** (see above).
