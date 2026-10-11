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

**Comparison chain**:
Two or more comparisons in one direction written together, such as
`a < b <= c`, meaning each adjacent pair holds.

**`unit`**:
The type of an expression that produces no value.
_Avoid_: void

**`never`**:
The type of an expression that does not finish, such as `return`; it matches
any type.

**Panic**:
The program stopping at runtime with `sisu: panic at <file>:<line>:<col>:
<message>` on stderr and exit code 101.
_Avoid_: trap, crash, abort

## Arrays

**Array**:
A fixed-length sequence of values of one type, written `[T]`. An array is a
value: binding it, passing it or storing it behaves as a copy.
_Avoid_: list, vector (a growable collection, milestone 11)

**Copy-on-write**:
How arrays behave as values without copying eagerly: holders share one block,
and a write to an element copies the block first when another holder exists.
_Avoid_: lazy copy

**Last use**:
A read of a local after which the local is not read again before it is
reassigned or goes out of scope. A last use hands over its reference instead
of sharing it, so a write that follows can skip the copy.
_Avoid_: move (the compiler's name for the node, not the language rule)

**Range**:
The `a..b` in a `for` header, covering `a` up to but not including `b`. It is
not a value and appears nowhere else.

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

**Poison type**:
The type the checker gives an expression that already failed a rule, so that
one mistake produces one diagnostic.
_Avoid_: error type (ambiguous with diagnostics)

**Reference count**:
The number of references to an object; the object is dropped when it reaches
0.
_Avoid_: refcount, retain count

**Weak reference**:
A reference, held in a `weak` field, that does not keep its object alive and
reads as `None` once the object has been dropped.
_Avoid_: unowned

**Owned reference**:
A reference that its holder must release exactly once. Every expression of
class or array type yields one.
