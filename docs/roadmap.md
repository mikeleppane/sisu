# Roadmap

Sisu grows one milestone at a time. Each milestone ends with a program that
proves it works. Its design is settled in a working spec before work starts.
The specs are not published. The decisions they settled are listed below,
and the lasting ones are recorded as ADRs in [adr](adr).

## Milestones 1 to 6

These are adopted. Together they give Sisu a working imperative core.

| # | Milestone | Done when | Status |
| --- | --- | --- | --- |
| 1 | Number crunching: `i64`, `bool`, functions, `if`, `while`, recursion | Fibonacci and a prime counter print correct answers | Done |
| 2 | Classes, methods, heap, reference counting, optionals with `?.` and `??`, `break` and `continue` | A linked list and a binary tree run clean under Valgrind | Done |
| 3 | Arrays and a byte type | A prime sieve prints correct answers | Planned |
| 4 | Strings | A program reverses and compares strings | Planned |
| 5 | Input and output | A word-count tool reads standard input | Planned |
| 6 | Floats | A Mandelbrot set prints as text | Planned |

### Decided in milestone 1

These came out of the review of the milestone 1 spec and are now part of it:

- `let` immutable by default, `var` mutable (ADR 0002), with local type
  inference.
- `if` is an expression.
- No semicolons; statements end at newlines (ADR 0001).
- One built-in `print(e)` for `i64` and `bool` instead of `print_int`.
- Compound assignment `+= -= *= /= %=`, desugared by the checker (moved from
  the parser in milestone 2).
- Digit separators: `100_000`.
- Diagnostics in the style of rustc: the code underlined, secondary labels
  such as "declared with `let` here", and a `help:` line.
- `break` and `continue` scheduled for milestone 2.
- From milestone 2, a line starting with `.` continues the previous line, so
  method chains can be written one call per line (ADR 0001).

### Decided in milestone 2

These came out of the milestone 2 design and are now part of its spec:

- Optionals for every type, `i64?` and `bool?` included, with `?.`, `??` and
  `None` as the empty value. `None` becomes the `Option<T>` case in
  milestone 8.
- `if let` and `while let` for optionals, brought forward from milestone 7.
- `==` compares classes structurally; `a is b` compares identity.
- The checker reports every error in one run, using a poison type.
- The checker builds a typed, desugared tree (`tir`) and codegen reads only
  that.
- Reference counting (ADR 0005).

## Beyond milestone 6

Status: proposal, 2026-10-09. Nothing from here on is decided until it lands
in a milestone spec or an ADR.

The rest of this document collects what comes after milestone 6, and the
ergonomic features that should be folded into milestones 2 to 6 while they
are still cheap.

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
- Memory is managed for you (reference counting, ADR 0005).

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

## Fold into milestones 2 to 6

Each item is cheap when its milestone lands and expensive to retrofit:

| Milestone | Add | Why |
| --- | --- | --- |
| Not scheduled | Reserve `import` and `pub` as keywords. Planned for milestone 2, it did not land. | A program that names a variable `import` would break when modules land in milestone 11 |
| 3 | Array literals `[1, 2, 3]` | Data reads like data |
| 3 | `for x in xs` and ranges `0..n` | Most loops stop needing indexes |
| 3 | Top-level `const NAME = expr`, where `expr` uses only literals, operators and other constants; the checker folds it to a value | Named limits, such as a sieve size, without global variables |
| 4 | String interpolation `"x = {x}"` | The biggest single win for real programs |

### Portability rules

`sisuc` already compiles for the machine it runs on: codegen asks LLVM for the
host triple and takes the data layout from it. Two rules keep it that way, so
that new targets stay cheap:

- From milestone 2, take sizes and alignments from the target data layout.
  Never write a pointer size as `8`.
- Runtime functions take and return only integers, `bool`, floats and
  pointers, never structs by value. How a struct is passed differs between
  x86-64, ARM64 and Windows. Clang applies those rules for C, but LLVM does
  not, so `sisuc` would have to implement them for each target.
  `sisu_panic(msg, len)` already follows this rule.

## Early tooling

These help you learn from the compiler and read Sisu code. Each one is small,
so it does not wait for the later goals:

- One `sisu` binary with subcommands, as with the `go` and `zig` commands.
  One binary means the compiler and the build tool can never be different
  versions, unlike Cargo and `rustc`. Planned for milestone 2, it did not
  land and is not scheduled yet. It renames `sisuc` to `sisu` and adds two
  subcommands:
  - `sisu build` does what `sisuc` does today, with the same flags.
  - `sisu run file.si` compiles to a temporary executable and runs it.

  Later subcommands are `sisu check` (type checking only), `sisu fmt`,
  `sisu test` and `sisu lsp`. No manifest at first: a single file, or a
  directory whose entry point is `main.si`, builds without one. A `sisu.toml`
  arrives only when a project needs dependencies.
- A `-O2` flag, added in milestone 2. It runs LLVM's `default<O2>` pass
  pipeline in place of `mem2reg`, both for `--emit ir` and for the
  executable. Without the flag, only `mem2reg` runs, as in milestone 1.
  Comparing the IR with and without the flag shows what the optimizer
  removes.
- `--emit ir-raw`, added in milestone 2. It prints one module before any pass
  runs, so the output is a valid `.ll` file for LLVM's `opt` tool. (`--emit ir`
  prints two modules, before and after the pass pipeline.) With `opt` you can watch
  the optimizer one pass at a time, with no further compiler code:
  - `opt -S -passes='mem2reg,simplifycfg,instcombine' x.ll` runs the named
    passes in order.
  - `--print-changed` prints the IR after each pass that changed it.
  - `-passes=dot-cfg` writes each function's control-flow graph as a `.dot`
    file for Graphviz.
  - `--opt-bisect-limit=N` runs only the first N passes, to find the pass
    that breaks a program.

  `opt` must be version 22, because an older `opt` may not parse the IR.
  `scripts/install-llvm.sh` does not keep `bin/opt`. Decide whether it should,
  checking the GitHub Actions cache limit first.
- A TextMate grammar for syntax highlighting in VS Code, from milestone 3,
  once classes and arrays fix most of the syntax. It is one JSON file and
  needs no compiler changes.
- ARM64 Linux and macOS on ARM64 as hosts. LLVM publishes `Linux-ARM64` and
  `macOS-ARM64` builds, so `scripts/install-llvm.sh` picks the download by
  `uname -s` and `uname -m`, and CI gets a job for each. The compiler should
  need no changes, but this is untested. On macOS, `cc` is Apple's clang, so
  tests that assume GNU tools or ELF files break. LLVM publishes no build for
  Intel Macs. Check which GitHub Actions runners the repository can use first.

## Proposed milestones 7 to 12

| # | Milestone | Done when |
| --- | --- | --- |
| 7 | Enums with payloads, `match` with destructuring and an exhaustiveness check, `if let` with enum patterns | The shape sample above runs; a non-exhaustive `match` is a compile error |
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

### Modules (milestone 11)

- One file is one module, and its path names it: `src/geometry/shapes.si` is
  `geometry.shapes`. A parent file needs no `mod shapes` declaration, as in
  Rust. A directory is not a package either: in Go, a file uses names from
  sibling files without importing them, so it cannot be read on its own.
- `import geometry.shapes` lets code call `shapes.area(s)`.
  `import geometry.shapes.{area, Shape}` brings those names in without a
  prefix.
- Names are private to their module unless marked `pub`.
- Modules may import each other in a cycle. The checker sees the whole
  program at once, so this costs nothing.
- Codegen puts the whole program into one LLVM module. Each symbol is
  prefixed with its module path; `main` keeps its name. Separate compilation
  and build caching wait until builds are slow.
- A small prelude is visible in every file without an import: `print`,
  `Option` and `Result`.

## Later and stretch goals

These do not make code nicer to write, so they wait:

- Weak references or a cycle collector. Reference counting leaks cycles, so a
  doubly linked list or a parent-pointer tree leaks without one. Decide before
  milestone 11 grows data structures that create cycles.
- C FFI: `extern fn` to call libc or a C library.
- A test runner: `test` blocks and `sisu test`.
- Parser error recovery, so a syntax error does not hide the next one.
- Lint checks as compiler warnings, as in Go's `vet` and `rustc`'s built-in
  lints, with no separate tool and no configuration file:
  - an unused variable or import;
  - code after `return`, `break` or `continue`, which never runs;
  - a `var` that is never reassigned ("use `let`"), as Swift and Kotlin warn;
  - comparing a value that is not optional with `None`, which is always
    `false`. Milestone 2 makes it an error until lints exist.

  An attribute such as `@allow(unused)` could later silence one declaration.
- Debug info (DWARF), so `gdb` steps through Sisu source.
- A mid-level IR between `tir` and LLVM IR, for optimizations such as
  reference-count elision. `tir` itself only records what the checker
  understood; it does not optimize.
- Link-time optimization with the runtime. Build `sisu-runtime` as LLVM
  bitcode and link it into the program's module, so LLVM can inline runtime
  calls such as the reference-count increment and decrement.
- A formatter, `sisu fmt`, after milestone 7, once `enum` and `match` have
  fixed most of the syntax. It has one style and no options, as with `gofmt`
  and `zig fmt`. It must keep comments, but the lexer drops them today
  (`crates/sisuc/src/lexer.rs`). As `gofmt` does, the lexer would record each
  comment's span, and the formatter would write it back next to the nearest
  syntax-tree node. It keeps single blank lines between statements, found
  from the spans. Formatting twice must change nothing, and the formatted
  file must parse to the same syntax tree.
- A language server, `sisu lsp`, reusing the lexer, parser and checker.
- Windows. There is no `cc`, so `sisuc` links with `link.exe` or `lld-link`.
  Programs are `.exe` files in the COFF object format. LLVM ships for Windows
  as an installer and a `clang+llvm` archive, so `llvm-sys` needs its own
  setup.
- Cross-compiling with `--target <triple>`. LLVM must initialize every target,
  not only the native one. The runtime must be built for the target
  (`cargo build --target`), and linking needs a cross linker and the target's
  system libraries.
- Calls to plain functions in a `const` initializer, for lookup tables. LLVM
  already folds constant arithmetic, so this adds expressiveness, not speed.
  Running the call through LLVM's JIT reuses codegen, so compile time and run
  time cannot disagree; a tree-walking interpreter would be a second copy of
  the semantics. Open points:
  - The JIT runs on the host, which can differ from the `--target`.
  - Only scalars at first, then strings. Heap objects would have to be
    serialized into globals.
  - No I/O at compile time, so builds stay reproducible.
  - A step or time limit, so a loop that never ends cannot hang the compiler.
- `@inline` and `@noinline` on functions. They set LLVM's `alwaysinline` and
  `noinline` attributes, as codegen already sets `noreturn` on `sisu_panic`.
  An attribute alone changes nothing: without `-O2`, `sisuc` must run
  `always-inline` before `mem2reg`. `default<O2>` inlines `alwaysinline`
  functions and respects `noinline`. Avoid an `inline fn` keyword, which in
  Kotlin inlines lambda arguments. `@inline` on a recursive function is an
  error. From milestone 9, calls through closures cannot be inlined. At
  `-O2`, LLVM already inlines small functions, so this waits for a need.
- Concurrency: threads, atomic reference counts, channels.
- Self-hosting: the Sisu lexer and parser written in Sisu.

## Deliberately avoided

- Lifetimes and a borrow checker. Reference counting keeps Sisu in the Swift
  and Kotlin ergonomic camp.
- Header files.
- Unrestricted operator overloading.
- Inheritance. A `class` is a reference type, not a hierarchy. Composition,
  enums (milestone 7) and interfaces (milestone 8) cover polymorphism, as in
  Rust and Go.
- Zig-style `comptime`, with types as compile-time values, and macros.
  Generics and interfaces (milestone 8) cover the same ground, and errors
  stay plain when type checking does not depend on running code.
- Formatter options. One style ends style debates.
- A separate linter. Lints are compiler warnings.
- `import *`. A reader must always see where a name comes from.
- A package registry, for a long time. A registry means hosting, security
  and name squatting. If dependencies come, path and Git dependencies come
  first.

## Open decisions

- **Same-scope shadowing.** Milestone 1 allows redeclaring a name only in an
  inner scope. Rust also allows `let n = n.trim()` in the same scope, which
  pays off once strings change a value's type. Decide in milestone 4; lifting
  the error breaks no program.
- **Generic arguments and `<`.** `f(a < b, c > d)` reads as two comparisons
  or as one generic call. Decide the syntax in milestone 8.
- **Weak references or a cycle collector** (see above).
- **Attribute syntax.** A marker on a declaration, such as `@inline`, needs
  one general form: `@name` or `@name(args)` on the line before it, as in
  Swift and Kotlin. Decide the form before the first attribute lands, so `@`
  is not taken for something else. Later uses could include `@noinline`,
  `@deprecated("...")` and `@test`.
- **What a string is.** LLVM sees only bytes, so Sisu decides this itself.
  Milestone 1 already reads UTF-8 source with ASCII identifiers. Decide before
  milestone 4:
  - whether strings are stored as UTF-8;
  - whether `len()` counts bytes or characters (Unicode scalars);
  - whether `s[i]` exists at all. Swift has no integer indexing on strings,
    because it is slow and misleading for text that is not ASCII.
