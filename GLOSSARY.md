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
variable, and later a field or an array element.
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
