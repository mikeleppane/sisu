# Area: runtime soundness

`crates/runtime/src/lib.rs` is the only hand-written `unsafe` code every
program links. Codegen calls it through `extern "C"` declarations that the
Rust compiler never sees, so a signature mismatch compiles cleanly and
breaks at run time.

## Probes

- Run the runtime's tests under Miri (toolbox, "Runtime under Miri"): the in-process tests, then every case of every `*_child` test with its env var. Any `Undefined Behavior` is a finding.
- Read each `unsafe` block and each `// SAFETY:` comment. For each, state the precondition and find the codegen call sites (`codegen/mod.rs`, `codegen/rc.rs`, `codegen/array.rs`) that must meet it. A precondition no caller is checked against is a lead; a program that breaks it is a finding.
- Compare every runtime export with its declaration in codegen: parameter types and widths, `bool` passing (`zeroext`), `usize` against `i64`, `noreturn` against `-> !`, and the `len` of each panic message against the bytes codegen emits.
- `sisu_alloc` and `sisu_free` get the same size and alignment for each block: run the layout shim (toolbox, "Memory") on objects, arrays of every element type, empty arrays, `[x; 0]`, and arrays that copy-on-write copied.
- `LIVE` counter: every allocation counts and every free uncounts, copy-on-write copies included. A `--rc-checks` program with balanced counts exits 0; one that keeps a block alive exits 102.
- Output: `sisu_print_int` and `sisu_print_bool` when stdout is closed (`$D/p >&-`) or a full pipe; `sisu_panic` when stderr is closed (`$D/p 2>&-`). The runtime tests one such case; find the others. The `SIGPIPE` exit is ruled in [finding.md](../finding.md).

## Not a finding

Miri's "unsupported operation" on a test that spawns a process is a tool
limit; run its child case directly instead.
