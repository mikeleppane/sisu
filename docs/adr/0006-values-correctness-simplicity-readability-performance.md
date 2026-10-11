# Correctness, then simplicity, then readability, then performance

Sisu's goal is that people are productive in it because things just work.
Four values serve that goal, in this order, and a design choice that trades
one for another follows the order:

1. **Correctness.** A program never produces a silent wrong answer. Failures
   are loud and name the source line. Nothing changes behind the reader's
   back: no hidden aliasing, no implicit conversions.
2. **Simplicity.** Few concepts, rules without special cases, one obvious way
   to do a thing, and every effect visible where it happens.
3. **Readability.** Little punctuation, code that reads top to bottom, and
   errors in plain words (ADR 0004).
4. **Performance.** Predictable and good enough, in the range of Swift and
   Kotlin. It comes only from optimizations that do not change what a program
   means.

Convenience, such as truthiness or implicit conversions, is welcome only when
it costs none of the first three.

Simplicity ranks above readability because agents now write and read much of
the code. Punctuation costs an agent little; special cases, two ways to do one
thing and effects that happen elsewhere are what cause its mistakes, and what
slow a human reviewing its work. A strict checker and plain-word errors matter
more for the same reason: an agent reads the error and fixes the code.

## Considered options

- **Productivity as a ranked value**: every feature claims it, so it cannot
  settle a trade-off. It is the goal the four values serve.
- **Readability above simplicity**: the order of ADR 0004's principles, chosen
  when people were the main readers.
- **Robustness as its own value**: as used here it means "fail loudly instead
  of going wrong quietly", which is correctness seen from the user's side.

## Consequences

- Earlier decisions fit the order: panics instead of wrapping (ADR 0003, a
  check on every `+` for correctness), reference counting instead of a borrow
  checker (ADR 0005, simplicity over performance).
- Where correctness costs readability, correctness wins: an effect must show
  in the code even if that adds a word.
- Every new feature names the value it serves and what it costs the values
  above it.
