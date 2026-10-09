# Integer overflow panics instead of wrapping

Overflow in `+ - *` and unary `-`, division by zero and `i64::MIN / -1` stop
the program with a panic that names the source location. We chose this over
wrapping (Go, Java) so a wrong answer can never be silent, and because it is
the reversible choice: explicit wrapping operators can be added later without
breaking any program, while switching from wrapping to panicking would break
every program that relies on wrapping. See the milestone 1 spec, "Arithmetic
panics".

## Considered options

- **Wrap**: the simplest IR and the fastest code, but `fib(93)` prints a
  negative number and the bug shows up far from its cause. Division by zero
  would still need a check.
- **Panic in debug builds, wrap in release (Rust)**: needs a build flag, and a
  test can pass in one mode and fail in the other.

## Consequences

- Every `+ - *` costs an overflow intrinsic, a branch and a panic block, which
  `--emit ir` shows.
- Hashing and random number generation will need explicit wrapping
  operations, added in the milestone that first needs them.
