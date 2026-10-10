# Sisu

Sisu is a small, statically typed language compiled to native code. These are
the terms its spec, compiler and docs use.

## Language

**Binding**:
A name introduced by `let`, `var` or a function parameter.
_Avoid_: variable (for a `let` or a parameter)

**Variable**:
A binding declared with `var`, the only kind that can be reassigned.
_Avoid_: mutable binding

**Place**:
An expression that names storage and can stand on the left of `=`: a
variable or a `var` field, and later an array element.
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

## Classes and objects

**Class**:
A named type whose values are references to objects with the same fields and
methods. Sisu classes have no inheritance.
_Avoid_: struct, record (a value type, milestone 10)

**Object**:
One heap-allocated value of a class, shared by every reference to it.
_Avoid_: instance

**Reference**:
A value of class type: it refers to an object, and copying it copies the
reference, not the object.
_Avoid_: pointer, handle

**Field**:
A named, typed slot in every object of a class, declared with `let` or `var`.
_Avoid_: property, member variable, attribute

**Method**:
A function declared inside a class whose first parameter is `self`, called as
`receiver.name(args)`.
_Avoid_: member function

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

**Owned reference**:
A reference that its holder must release exactly once. Every expression of
class type yields one.
