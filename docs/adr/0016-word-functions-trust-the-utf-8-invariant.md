# The word functions trust the UTF-8 invariant

Status: accepted, 2026-10-14. It amends the first rule of
[ADR 0014](0014-strings-are-utf-8-values.md), which says the runtime aborts
if a compiler bug breaks the invariant.

`sisu_words_count` and `sisu_word_next`, which `words()` and
`s.words().len()` call, do not check that a string's bytes are valid UTF-8 in
a release build. They trust ADR 0014's invariant. A debug build of the
runtime still checks, once per call of `sisu_words_count`. If a compiler bug
ever produced a string that is not valid UTF-8, these two functions would
have undefined behaviour in a release build, where they aborted before. The
other runtime functions that decode a string, `sisu_utf8_count` and
`sisu_utf8_decode`, still check and abort.

The check cost a full pass over every line that the `wc` benchmark counts,
and the invariant already holds: the lexer validates literals, `char(n)` and
`string(cs)` take only scalar values, and `read_line` and `args` repair their
input. A runtime function that later wants the same trade names it here.

## Considered options

- **Keep the check in release builds**: the abort stays, but every
  `words()` pays for a pass that can only fail on a compiler bug.
- **Check in `sisu_word_next` too, in debug builds**: it would rescan the
  whole string for every word, which makes `words()` quadratic in a debug
  build.

## Consequences

- A compiler bug that breaks the invariant shows up as an abort in a debug
  build, or as wrong output or a crash in a release build of a program that
  calls `words()`.
