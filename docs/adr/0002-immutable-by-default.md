# Bindings are immutable by default

`let` introduces an immutable binding and `var` a reassignable one; function
parameters are immutable. The checker rejects assignment to a `let` or a
parameter and warns about a `var` that is never reassigned, so the keyword
always tells the reader whether a name can change. Two keywords keep the
common cases short, as in Swift and Kotlin. See the milestone 1 spec,
"Bindings: immutable by default".
