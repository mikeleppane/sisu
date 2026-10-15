# Sisu

Sisu is a small, statically typed language compiled to native code. These are
the terms its spec, compiler and docs use.

## Language

**Binding**:
A name introduced by `let`, `let mut` or a function parameter.
_Avoid_: variable (for a `let` or a parameter)

**Variable**:
A binding declared with `let mut`, the only kind that can be reassigned.
_Avoid_: mutable binding, `var` (the keyword before milestone 4)

**Place**:
An expression that names storage and can stand on the left of `=`: a
variable, a `mut` field, or an element of an array held in a place, such as
`grid[i][j]`.
_Avoid_: lvalue

**Block value**:
The value of a block's last statement when that statement is an expression;
otherwise the block has no value.

**Unused value**:
The value of an expression statement that nothing uses. The compiler warns
about one; `let _ = e` discards a value on purpose.
_Avoid_: dropped value (dropping is what a reference count does)

**Comparison chain**:
Two or more comparisons in one direction written together, such as
`a < b <= c`, meaning each adjacent pair holds.

**`unit`**:
The type of an expression that produces no value.
_Avoid_: void

**`never`**:
The type of an expression that does not finish, such as `return`; it matches
any type.

**Conversion**:
A call written with a type's name, `T(x)`, that turns a value into an
adjacent type, such as `u8(n)` or `i64(c)`. A conversion that can fail
returns `T?`.
_Avoid_: cast, fallible conversion, `as`

**Panic**:
The program stopping at runtime with `sisu: panic at <file>:<line>:<col>:
<message>` on stderr and exit code 101. A failed write to standard output or
read from standard input has no location: `sisu: panic: <message>`.
_Avoid_: trap, crash, abort

**Prelude**:
The names visible in every file without an import.

**RC check**:
A check that a program built with `sisuc --rc-checks` makes on its reference
counts: no retain or release of a count below 1, and no object still
allocated at a normal exit. A failed check prints `sisu: rc check: <what>`
on stderr and exits with code 102.

## Arrays

**Array**:
A fixed-length sequence of values of one type, written `[T]`. An array is a
value: binding it, passing it or storing it behaves as a copy.
_Avoid_: list, vector (a growable collection, milestone 11)

**Copy-on-write**:
How arrays and strings behave as values without copying eagerly: holders
share one block, and a write (to an array element, or `+=` on a string)
copies the block first when another holder exists.
_Avoid_: lazy copy

**Last use**:
A read of a local after which the local is not read again before it is
reassigned or goes out of scope. A last use hands over its reference instead
of sharing it, so a write that follows can skip the copy.
_Avoid_: move (the compiler's name for the node, not the language rule)

**Range**:
The `a..b` in a `for` header, covering `a` up to but not including `b`. It is
not a value and appears nowhere else.

## Text

**`string`**:
A sequence of bytes that is always valid UTF-8. A string is a value, like an
array. Two strings are equal when their bytes are equal; there is no Unicode
normalisation.
_Avoid_: str, text

**`char`**:
One Unicode scalar value: a code point that is not a surrogate. It is written
between single quotes, as in `'a'` or `'\u{1F600}'`. It compares by code point
and has no arithmetic: `i64(c)` and `char(n)` convert.
_Avoid_: character (readers take it to mean what they see, which can be
several scalar values)

**Interpolation**:
A `{expr}` inside a string literal. The expression is evaluated and turned
into text, and the whole literal becomes one new `string`.
_Avoid_: formatting, templating

**Format spec**:
The `[align][width][.precision]` after a `:` that ends an interpolation, as
in `{x:>8.2}`.
_Avoid_: format string, format specifier

**Shadowing**:
Declaring a `let` or `let mut` with the name of a binding that is already
visible, in the same scope or an outer one. The new binding replaces the old
one from its declaration onwards, and its right-hand side still sees the old
one.
_Avoid_: redeclaration, rebinding

**Constant**:
A named value declared at the top level with `const` and folded at compile
time. It is typed, and a local binding with its name shadows it. It is
neither a `let` binding nor a literal: each use becomes the literal its value
folds to.
_Avoid_: global, static

**Reserved keyword**:
A word that the lexer already treats as a keyword for a feature that comes in
a later milestone, such as `match` or `import`. Using one as a name is an
error, so a program written today does not break when the feature lands.
_Avoid_: future keyword

## Classes and objects

**Class**:
A named type whose values are references to objects with the same fields and
methods. Sisu classes have no inheritance.
_Avoid_: struct, record (a value type, milestone 10)

**Record**:
A named type with fields and methods whose values are not shared: binding,
passing or storing one behaves as a copy, as an array does. Records come in milestone 10.
_Avoid_: struct, data class

**Object**:
One heap-allocated value of a class, shared by every reference to it.
_Avoid_: instance

**Reference**:
A value of class type: it refers to an object, and copying it copies the
reference, not the object.
_Avoid_: pointer, handle

**Field**:
A named, typed slot in every object of a class or value of a record, declared
on its own line in the type's body. A class field marked `mut` can be
reassigned; a bare one is set by the constructor and never changes.
_Avoid_: property, member variable, attribute

**Method**:
A function declared inside a class or record whose first parameter is `self`,
called as `receiver.name(args)`.
_Avoid_: member function

**Mutating method**:
A method of a record or an array that takes `mut self`, such as `push`, and
changes the value in the place it was called on. That place must be able to
change.
_Avoid_: `inout`, `mutating`

**Receiver**:
The expression before `.` or `?.` in a field read or a method call: the `a`
in `a.f` and `a.m()`.

**Constructor**:
The call `Class(field: value, ...)` that creates an object, naming every field
in declaration order.
_Avoid_: initializer, init

**Drop**:
Freeing an object when the last reference to it goes away, after releasing
the references in its fields.
_Avoid_: deinit, destructor, finalizer

**Structural equality**:
What `==` means on classes: two objects are equal when each pair of fields is
equal.

**Identity**:
Whether two references refer to the same object, tested with `is`.
_Avoid_: reference equality, `===`

## Optionals

**Optional**:
A value of type `T?`, holding either a `T` or `None`.
_Avoid_: nullable, maybe

**`None`**:
The optional that holds no value.
_Avoid_: nil, null

**Unwrap**:
Getting the `T` out of a `T?` when it holds one, with `if let`, `while let`,
`?.` or `??`.

## Compiler

**Typed tree (`tir`)**:
The program as the checker understood it: names resolved, every expression
typed, sugar such as `while` and `?.` lowered. Codegen reads only this.
_Avoid_: IR (that is LLVM's), HIR

**Symbol**:
A name as the parser and checker hold it: a small integer that stands for the
name's text. The lexer interns each name, so equal names get the same symbol.
_Avoid_: atom, interned string

**Poison type**:
The type the checker gives an expression that already failed a rule, so that
one mistake produces one diagnostic.
_Avoid_: error type (ambiguous with diagnostics)

**Diagnostic**:
An error or a warning the compiler reports about the source. An error stops
the build; a warning does not.
_Avoid_: report, message

**Diagnostic code**:
The stable name of one kind of diagnostic, such as `E0007` or `W0002`. A code
is never reused, and the same code always reads the same way.
_Avoid_: error code (warnings have codes too)

**Help**:
Advice in a diagnostic that cannot be written as an edit, such as "optionals
do not nest".

**Fix**:
An edit to the source, with a message, that resolves a diagnostic. A fix is
either certain or a guess.
_Avoid_: suggestion, quick fix

**Explanation**:
The long-form page for one diagnostic code: the rule, why Sisu has it, and a
broken and a fixed example.
_Avoid_: error index entry

**Broken item**:
A function, class or method that contains a syntax error. Its body is not
checked, and a use of it is never reported as unknown or mistyped.

**Cascade**:
A diagnostic that exists only because of another one. Sisu never reports
one.
_Avoid_: follow-on error, knock-on error

**Foreign idiom**:
A habit from another language, such as `var` or `len(xs)`, that has its own
diagnostic code naming the Sisu way.

**Reference count**:
The number of references to an object; the object is dropped when it reaches
0.
_Avoid_: refcount, retain count

**Weak reference**:
A reference, held in a `weak` field, that does not keep its object alive and
reads as `None` once the object has been dropped.
_Avoid_: unowned

**Counted type**:
A type whose values are shared blocks carrying a reference count: classes,
arrays and strings, and optionals of them. Copying such a value copies a
pointer and raises the count; `i64`, `u8`, `bool` and `char` are copied whole and are not counted.
_Avoid_: reference type, heap type

**Owned reference**:
A reference that its holder must release exactly once. Every expression of a
counted type yields one.
