# Labels belong to the declaration

Status: accepted, 2026-10-15. Milestone 7 builds it. Milestone 10's named
arguments and records may extend it.

A variant's payload either labels every field or labels none, and the labels
are written once, in the declaration:

```
enum Shape {
    Circle(radius: i64)
    Rect(w: i64, h: i64)
    Dot(i64, i64)
}
```

- **A payload labels every field or none.** `Move(i64, to: i64)` is an error:
  "label every field of `Move` or none".
- **A labeled variant is built with every label, in declared order**, exactly
  as a class constructor is today: `Shape.Rect(w: 2, h: 3)`. An unlabeled
  variant is built by position: `Shape.Dot(1, 2)`. A label on an unlabeled
  variant is an error.
- **Each declaration owns its labels.** A class `Size { width, height }` does
  not constrain a variant's `w` and `h`. A variant that holds a whole `Size`
  (`Boxed(Size)`) is built with `Size`'s own labels inside:
  `Shape.Boxed(Size(width: 2, height: 3))`.
- **A pattern binds by position and never takes labels**, on a labeled
  variant too: `Rect(width, height) => width * height`. The names in a
  pattern are new locals, not the declared labels.
- **The errors are the class constructor's**, with the variant's name
  (`Shape.Rect`) in place of the class's: a missing label, labels out of
  order, an unknown label, a wrong number of fields. One rule has one
  message; wording that changes for variants changes for classes too.

In milestone 7 labels are required and in declared order. Milestone 10's
named arguments, which let a call give labels in another order, and its
records, whose patterns may take labels, can extend these rules without
changing what a milestone 7 program means.

## Considered options

- **Labels in patterns (`Rect(w: x, h: y)`)**: it names the field a binding
  takes. But it is a second spelling for the same match, it needs the
  order-free label rules of named arguments, and position already tells the
  binding apart. It waits for milestone 10.
- **Labels in any order at a call**: this is named arguments. Doing it for
  variants alone would make variants and class constructors differ, so it
  waits for milestone 10, which does both.
- **Some fields labeled, some not (Swift)**: a reader must remember which
  fields take a label. All or none keeps the call site uniform.
- **Labels owned by a type, shared between declarations**: a class's labels
  would bind every variant that holds it, and an edit to the class would
  break variants that never named it.

## Consequences

- A variant of one field with a label, `Circle(radius: i64)`, is built
  `Shape.Circle(radius: 1)`; the label is part of every construction.
- A pattern on a labeled variant reads the same as on an unlabeled one, so
  adding labels to a declaration breaks no pattern.
- Milestone 10 can add labels to patterns, or relax the order at a call,
  without breaking a program that follows this ADR.
