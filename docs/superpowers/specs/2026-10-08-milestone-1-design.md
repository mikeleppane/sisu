# Milestone 1: number crunching

Status: approved, 2026-10-09. Plan:
`docs/superpowers/plans/2026-10-09-milestone-1.md`.

## Goal

`sisuc` compiles real Sisu source with `i64`, `bool`, functions, `if`, `while`
and recursion to a native executable. The milestone is done when these print
correct answers:

- `crates/sisuc/tests/programs/fib.sisu` prints `fib(30)`: `832040`.
- `crates/sisuc/tests/programs/primes.sisu` prints the number of primes below
  100 000: `9592`.

Sisu is a learning project, so every compiler stage is hand-written and
visible: no lexer or parser generators. Work that is not compiler design, such
as rendering diagnostics, uses a crate, checked with GitHits before it is
added.

## The language

```rust
// Counts the primes below 100 000.
fn is_prime(n: i64) -> bool {
    if n < 2 { return false }
    var d = 2
    while d * d <= n {
        if n % d == 0 { return false }
        d += 1
    }
    true
}

fn main() {
    var count = 0
    var n = 2
    while n < 100_000 {
        if is_prime(n) { count += 1 }
        n += 1
    }
    print(count)
}
```

### Statements end at newlines

There is no `;` in Sisu (ADR 0001). The lexer inserts a `Newline` token at a
line break, or at end of file, when both hold:

- the previous token is an identifier, an integer, `true`, `false`, `return`,
  `)` or `}` (Go's rule);
- no bracket is open, or the innermost open bracket is `{` (Python's rule,
  stopped at blocks). The lexer keeps a stack of open brackets, so an
  expression inside `( )` can span lines while a block inside it still
  separates its statements.

Comments are skipped and do not affect insertion. Four layout rules follow;
the first three are Go's:

1. A continuation line cannot start with an operator; end the previous line
   with it, or wrap the expression in parentheses.
2. `{` goes on the same line as `fn`, `if` or `while`.
3. `} else {` stays on one line. The parser reports "`else` must be on the same
   line as the closing `}`" when `else` starts a line.
4. A line that starts with `-` followed by a space is an error: "a line
   cannot start with a binary `-`", with "help: to continue the previous
   line, end it with `-`; to negate, write `-x`". Without it,
   `let t = a` followed by a line `- b` compiles silently as two statements.
   The rule applies only where the line break ended a statement; inside
   `( )` or after an operator, `- b` continues the expression.

### Bindings: immutable by default

`let` and `var` follow ADR 0002.

- `let x = 0` binds an immutable value; `var n = 2` binds a reassignable one.
- Function parameters are immutable.
- The type comes from the initializer; `let x: i64 = 0` writes it out.
  Parameters and return types are always written.
- Assigning to a `let` or a parameter is an error: "cannot assign to `n`",
  with a second label at the declaration ("declared with `let` here") and
  "help: declare it with `var`". For a parameter, the label is "declared as a
  parameter here" and the help "copy it into a `var`: `var n = n`".
- A `var` that is never reassigned gets a warning: "`count` is never
  reassigned", with "help: declare it with `let`". Warnings print;
  compilation continues.
- `x += e` (and `-= *= /= %=`) means `x = x + e`, so the same rules apply.

### Types, expressions and control flow

- Types in source: `i64`, `bool`. Internally also `unit` (no value) and
  `never` (an expression that does not finish, such as `return` or an
  `if`/`else` whose branches all return; it matches any type).
- Operators: `+ - * / %`, unary `-`, `== != < <= > >=`, `&& || !`. `&&` and
  `||` short-circuit.
- `/` truncates toward zero and `%` takes the sign of the dividend:
  `-7 / 2 == -3`, `-7 % 2 == -1`.
- Comparisons chain in one direction: `a < b <= c` means `a < b && b <= c`,
  with `b` evaluated once. A chain uses only `<`/`<=` or only `>`/`>=`;
  `== !=` do not chain.
- `if` is an expression. A block's value is its last statement when that is an
  expression; otherwise the block has no value. `if` without `else` has no
  value. `if`/`else` has the type of its branches, which must match. Where no
  value is expected (the body of a function without `-> T`, a `while` body, an
  `if` without `else`), a block's value is discarded.
- `while cond { ... }` has no value. `return` exits early, with or without a
  value. `break` and `continue` come in milestone 2.
- A function without `-> T` returns no value. `fn main()` takes no parameters
  and returns no value; the process exits with 0.
- `print(e)` is built in. It takes one `i64` or `bool` and prints it
  (`true`/`false` for a `bool`) and a newline.
- Comments run from `//` to the end of the line.

### Arithmetic panics

Arithmetic panics rather than wrapping (ADR 0003). Overflow in `+ - *` and
unary `-`, division or remainder by zero, and `i64::MIN / -1` (or `% -1`) all
stop the program: it prints `sisu: panic at <file>:<line>:<col>: <message>` to
stderr and exits with 101. Messages: `integer overflow`, `division by zero`.

## Pipeline

```text
source text
  lexer.rs    lex(&str)          -> Vec<Token>
  parser.rs   parse(&[Token])    -> ast::Program
  check.rs    check(&Program)    -> Vec<Diagnostic>
  codegen.rs  compile(&Program)  -> Module, then mem2reg, then write_object
  link.rs     (unchanged)        -> executable
```

Support modules:

- `ast.rs`: `Function`, `Stmt`, `Expr` and `TypeExpr`. Every node has a
  `Span`.
- `diagnostic.rs`: `Span { start, end }` in byte offsets, and a `Diagnostic`
  in the style of rustc (see "Diagnostics").

### Diagnostics

A `Diagnostic` has:

- a severity, `error` or `warning`;
- a message: "cannot assign to `n`";
- a primary label: the span of the offending code, underlined `^^^`, with
  optional text;
- secondary labels: other spans that explain it, underlined `---`, such as
  "declared with `let` here" or "`(` opened here";
- an optional `help:` line that says how to fix it.

`diagnostic.rs` converts a `Diagnostic` to `annotate-snippets` 0.12 types
and lets that crate render it:

```text
error: cannot assign to `n`
 --> primes.sisu:4:5
  |
2 |     let n = 0
  |         - declared with `let` here
3 |     ...
4 |     n = 1
  |     ^^^^^ cannot assign twice
  |
  = help: declare it with `var`
```

Source positions:

- Line and column are 1-based. A column counts characters (Unicode scalar
  values), not bytes.
- `<file>`, in diagnostics and in panic messages, is the input path exactly as
  given on the command line.

The lexer and parser stop at the first error. The checker returns its
warnings and at most one error; an error stops compilation after everything
is printed. Reporting every error in one run comes later (see the roadmap).

### CLI

- `sisuc <input.sisu> <output>` compiles to an executable.
- `sisuc --emit tokens|ast|ir <input.sisu>` prints that stage's output to
  stdout. `tokens` prints one token per line as `line:col Kind text`. `ir`
  prints the module before and after `mem2reg`.
- `sisuc --check <input.sisu>` runs the lexer, parser and checker, prints
  errors and warnings, and exits with 0 or 1.

Each stage wires itself into `--emit` or `--check` as it lands, so no stage
leaves dead code. The current `sisuc <output>` mode, which compiles the fixed
`print_int(42)` program, stays until stage 4 replaces it with the real compile
path; `tests/hello.rs` becomes the program `hello.sisu`.

## Lexer

- Keywords: `fn let var if else while return true false`.
- `i64` and `bool` are identifiers; the checker resolves them. Identifiers are
  ASCII: `[A-Za-z_][A-Za-z0-9_]*`. Comments may hold any UTF-8.
- Integers are decimal. `_` may separate digits: `100_000`. A `_` that does not
  sit between two digits (`1__0`, `100_`) is an error: "`_` must sit between
  digits". A literal above `i64::MAX`, checked after removing the `_`s, is an
  error. Known limit: `-9223372036854775808` cannot be written.
- Symbols: `+ - * / % += -= *= /= %= == != < <= > >= && || ! = ( ) { } , : ->`.
  A lone `&` or `|`, or any other character, is an error.

## Parser

Recursive descent for items and statements, Pratt parsing for expressions.

```text
program  = { Newline } { function { Newline } } EOF
function = "fn" Ident "(" [ param { "," param } ] ")" [ "->" type ] block
param    = Ident ":" type
type     = Ident
block    = "{" { Newline } [ stmt { Newline { Newline } stmt } { Newline } ] "}"
stmt     = ("let" | "var") Ident [ ":" type ] "=" expr
         | "while" expr block
         | "return" [ expr ]
         | expr [ ( "=" | "+=" | "-=" | "*=" | "/=" | "%=" ) expr ]
primary  = Int | "true" | "false" | Ident [ "(" args ")" ] | "(" expr ")" | if
if       = "if" expr block [ "else" ( block | if ) ]
args     = [ expr { "," expr } ]
```

- Types go through one `parse_type()` that returns a `TypeExpr`. In
  milestone 1 it accepts only a name; `Node?`, `[i64]` and `List<T>` later
  change only this function.
- An expression statement followed by an assignment operator is an
  assignment. Its left side must be a place, which in milestone 1 is a bare
  name; anything else is an error: "cannot assign to this expression". Fields
  (`p.x`) and indexes (`a[i]`) become places in later milestones. The parser
  rewrites `x += e` to `x = x + e`, with the span of the whole statement, so
  the checker and codegen see only plain assignment.
- After `return`, a `Newline` or `}` means no value; anything else starts the
  value.
- At level 4 the Pratt loop collects a chain of comparisons into one
  `Compare { operands, ops }` node. Mixing `<`/`<=` with `>`/`>=` is an error:
  "a comparison chain must go in one direction".

| Precedence | Operators | Associativity |
| --- | --- | --- |
| 1 (lowest) | `\|\|` | left |
| 2 | `&&` | left |
| 3 | `== !=` | none: `a == b == c` is an error |
| 4 | `< <= > >=` | chain, in one direction |
| 5 | `+ -` | left |
| 6 | `* / %` | left |
| 7 | unary `- !` | prefix |
| 8 (highest) | call | postfix |

## Checker

The checker validates and builds no typed tree; codegen reads types from the
LLVM values it builds, which is enough for `i64` and `bool`.

1. Collect every function signature, so functions can recurse and appear in
   any order. Errors: a duplicate function name; a missing `main`; a `main`
   with parameters or a return type.
2. Check each body with a stack of scopes, one per block. Redeclaring a name in
   the same scope is an error; shadowing in an inner scope is allowed.

Rules:

- Conditions are `bool`. `+ - * / %`, unary `-` and `< <= > >=` take `i64`;
  `&& || !` take `bool`; `== !=` take two operands of the same type, `i64` or
  `bool`. Call arguments match the parameters in number and type.
- `print` takes exactly one argument of type `i64` or `bool`; otherwise:
  "`print` takes one `i64` or `bool`".
- Functions and bindings live in separate namespaces: a call looks up the
  function table, a bare name looks up the scopes. A function named `print` is
  a duplicate function; a repeated parameter name is a redeclaration.
- A value of type `unit` cannot be bound or used as an operand or argument.
- Assignment needs a `var` of the same type.
- A function's body value matches its return type; a `return` value matches it
  too.
- "unreachable code" is a warning for the first statement after a statement
  of type `never` in the same block; the trailing statements are still
  checked, and the block has type `never`. It is an error for a `never`
  expression used as an operand, argument, initializer, assigned value or
  condition. So `never` appears only as a statement or as a block's last
  expression, and codegen relies on that: once a statement has type `never`,
  the block ends.

## Codegen

- Declare every function first, then define them. User functions are named
  `@sisu.<name>`: C identifiers cannot contain `.`, so no Sisu function
  collides with a libc symbol. Sisu `main` is `@sisu.main` like any other
  function; codegen adds `define i32 @main()`, which calls it and returns 0.
- `let` bindings and parameters are SSA values with no alloca. Each `var` gets
  an `alloca` in the function's entry block, as `mem2reg` requires.
- `+ - *` and unary `-` use `llvm.s{add,sub,mul}.with.overflow.i64`. `/` and
  `%` check for a zero divisor and for `MIN / -1` before `sdiv`/`srem`. Each
  check branches to its own panic block, which calls
  `sisu_panic(msg, len)` with the full message as a constant string, then
  `unreachable`.
- `if`/`else` with a value: then, else and merge blocks and a hand-written
  `phi`. Each incoming block is the block where its branch ended, which a
  nested `if` moves. A branch that ends in `return` contributes no incoming
  edge. `&&`, `||` and comparison chains use the same pattern; a chain's
  middle operands are SSA values computed once and reused.
- `while`: condition, body and end blocks.
- `print` calls the runtime's `sisu_print_int` or `sisu_print_bool`, chosen by
  the argument's LLVM type.
- After building: verify the module, run `mem2reg` through the new pass
  manager (`Module::run_passes("mem2reg", …)` in inkwell 0.10), then
  `write_object`.

### Runtime

`sisu-runtime` gains:

- `sisu_panic(msg: *const u8, len: usize) -> !`, which prints
  `sisu: panic at <msg>` to stderr and exits with code 101;
- `sisu_print_bool(b: bool)`, which prints `true` or `false` and a newline.

## Testing

- Lexer: token kinds for given source, including `Newline` insertion after
  `)`, inside parentheses, inside a block inside parentheses, after a comment
  and at end of file; the error for a line starting with `-` and a space; digit separators and each bad
  `_` placement.
- Parser: an S-expression printer for `Expr`, so tests compare
  `1 + 2 * 3` with `(+ 1 (* 2 3))`, `a < b <= c` with `(< a b <= c)` and
  `x += 1` with `(= x (+ x 1))`; the mixed-direction chain error; `return`
  with and without a value.
- Diagnostics: a few tests compare the full rendered text, including a
  secondary label and a `help:` line.
- Checker: one test per rule, checking the message and `line:col`. For
  `print`: no argument, two arguments, and a `unit` argument
  (`fn g() {}` then `print(g())`).
- End to end: `crates/sisuc/tests/programs/<name>.sisu` with `<name>.out`, one
  `#[test]` per program. `fib`, `primes` and `hello` check stdout; `overflow`
  and `div_zero` check exit code 101 and the panic message on stderr.

## Build order

One PR per stage, each with an Obsidian concept note.

| Stage | Adds | Proof |
| --- | --- | --- |
| 1 | `diagnostic.rs` and the `annotate-snippets` dependency, `lexer.rs` | `--emit tokens` |
| 2 | `ast.rs`, `parser.rs` | `--emit ast` |
| 3 | `check.rs`, `--check` | `--check` reports type errors and warnings |
| 4 | codegen for functions, `let`, `if` and its phi, comparison chains, calls, `print`, arithmetic panics; `sisu_panic`, `sisu_print_bool`; the real CLI, replacing the fixed-program mode | `fib.sisu` prints `832040` |
| 5 | `var`, `while`, assignment and compound assignment, `mem2reg`, `--emit ir` | `primes.sisu` prints `9592` |

## Out of scope

`break` and `continue` (milestone 2), more than one source file, error
recovery (more than one error per run), a typed intermediate tree, and
building SSA without `mem2reg`. The last is a possible later exercise (Braun
et al., "Simple and Efficient Construction of SSA Form", 2013).
