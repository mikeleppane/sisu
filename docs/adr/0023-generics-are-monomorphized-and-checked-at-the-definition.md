# Generics are monomorphized and checked at the definition

Status: accepted, 2026-10-16. It settles how milestone 8 compiles and checks
generic code, and the syntax that writes it.

A generic function, class, enum, interface or type alias takes type
parameters in angle brackets after its name: `fn largest<T: Ord>(xs: [T])`,
`class Stack<T>`. Two rules decide how the compiler treats them.

- **Compiled by monomorphization.** Each distinct list of type arguments gets
  its own compiled copy, and only the copies a program reaches are compiled.
  An instantiation's symbol names its arguments: `sisu.largest<i64>`.
  `largest<i64>` compiles to the code of a `largest` written for `i64`.
- **Checked once, at the definition.** Inside a generic body, a value of type
  `T` supports only what `T`'s bounds give it: the bounds' methods, and the
  operators they map to. Anything else is an error at the definition, such as
  "`T` has no `<`", with the help "add a bound: `T: Ord`". A call that
  compiles never reports an error inside the generic body. A bound that a
  type argument does not meet is reported at the call.

Four more rules go with them.

- **Bounds combine with `&`.** `T: Shape & Ord`. A comma would read as a
  second type parameter.
- **Type arguments are written in angle brackets, and `<` starts a list only
  when a list follows.** After a name in an expression, `<` starts a
  type-argument list only if a well-formed list follows and its closing `>`
  is followed by `(` or `.`. Otherwise `<` is a comparison, so
  `f(a < b, c > d)` stays two comparisons. Swift and C# also look at what
  follows the closing `>`.
- **Type arguments are inferred locally, as in Swift.** From the arguments
  (`largest([3, 9, 2])`) and from the expected type
  (`let s: Stack<string> = Stack(items: [])`) only. A type argument that
  neither gives is an error asking for an annotation. Wider inference can
  come later without breaking a program.
- **An interface takes type parameters, associated types, or both.** Use an
  associated type when each type has one answer (`Source.Item`), and a type
  parameter when a type can conform several ways (`Convert<Fahrenheit>` and
  `Convert<Kelvin>`). A type conforms to an interface with the same arguments
  at most once.

## Considered options

- **Brackets, `[T]` (Go, Scala)**: no `<` rule needed. But array types and
  indexing already use brackets in Sisu, so `Stack[i64]` would read as an
  index and `[Stack[i64]]` as nested brackets.
- **Angle brackets with a rule for `<` (Swift, C#, Rust, Kotlin, Java)**:
  readers already know the shape. A comparison and a type-argument list look
  alike, and the parser must decide with the `<` rule. Rust avoids the
  question by requiring `::<` in an expression; Sisu follows Swift and C#
  instead, since explicit arguments are rare and inference covers most calls.
- **Checking each instantiation (C++ templates)**: a generic body needs no
  bounds, but a call that compiles can fail deep inside the body, with an
  error about code the caller did not write. Sisu checks the definition
  against its bounds, as Rust and Swift do, so an error points at the call
  or at the definition, never into an instantiation.
- **Boxing generic values (Java, Kotlin)**:
  one compiled copy serves every type, so code is smaller. But a value of
  type `T` goes through a table or a box, and a generic function is slower
  than its hand-written copy. ADR 0006 ranks performance below readability,
  but this cost would fall on every generic collection, and monomorphization
  leaves a program no slower than one written by hand for each type.
- **Associated types only (Rust's `Iterator`, Swift's protocols)**: one
  answer per conforming type, as for `Source.Item`. It cannot say that
  `Celsius` converts to two types. **Type parameters only (Java's
  `Comparable<T>`)**: say that, but `Source<T>` then lets one type be a
  `Source` of two item types, so `for x in it` could not know the type of
  `x`. Sisu allows both.

## Consequences

- A program's compiled size and compile time grow with the number of
  instantiations it reaches. Milestone 11's build caching is complicated by
  it: a generic function's copy is emitted in the module that uses it.
- A recursion that would instantiate infinitely many copies, such as
  `fn f<T>(x: T) { f([x]) }`, is an error naming the cycle, found by the
  checker before codegen.
- A generic definition's bounds are part of its signature. Adding a bound
  later breaks the callers that do not meet it.
- `where` clauses are not part of this milestone. A bound goes in the angle
  brackets, and an associated type may carry one (`type Item: Display`), so
  adding `where` later breaks no program.
