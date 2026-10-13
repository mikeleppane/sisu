# Benchmarks

These numbers compare Sisu with C, Rust, Go, Java and Python on small
single-threaded programs. They show where Sisu stands, not how fast it will
be. Each program uses the same algorithm in every language, written plainly,
with no per-language tuning.

## Results

Commit 7c8d29e for binary_trees and 9cbd44e for the others (the change
between them is the runtime's live count), 2026-10-12, Intel Core i9-14900K,
Linux 7.0. Each figure is the median of 5 runs, pinned to one CPU. Each table runs from fastest to slowest
wall time, with Sisu in bold.

### fib

| Language | Wall (s) | CPU (s) | Peak RSS (MiB) | Binary (KiB) | Build (s) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Rust | 0.17 | 0.17 | 1.9 | 329 | 0.06 |
| C | 0.18 | 0.18 | 1.4 | 14 | 0.04 |
| **Sisu** | **0.26** | **0.25** | **1.4** | **317** | **0.03** |
| Java | 0.27 | 0.26 | 40.3 | - | 0.19 |
| Go | 0.28 | 0.28 | 1.8 | 1483 | 1.88 |
| Python | 5.18 | 5.16 | 11.0 | - | - |

### sieve

| Language | Wall (s) | CPU (s) | Peak RSS (MiB) | Binary (KiB) | Build (s) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Rust | 0.18 | 0.18 | 49.4 | 329 | 0.14 |
| C | 0.20 | 0.20 | 48.6 | 14 | 0.05 |
| Go | 0.21 | 0.21 | 49.8 | 1483 | 0.19 |
| **Sisu** | **0.23** | **0.23** | **48.9** | **317** | **0.04** |
| Java | 0.24 | 0.23 | 89.1 | - | 0.20 |
| Python | 2.29 | 2.29 | 392.5 | - | - |

### binary_trees

| Language | Wall (s) | CPU (s) | Peak RSS (MiB) | Binary (KiB) | Build (s) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Java | 0.37 | 0.36 | 334.0 | - | 0.19 |
| C | 0.62 | 0.61 | 33.4 | 14 | 0.05 |
| Rust | 0.72 | 0.71 | 33.9 | 330 | 0.13 |
| **Sisu** | **0.85** | **0.85** | **33.4** | **317** | **0.05** |
| Go | 1.33 | 1.32 | 46.3 | 1487 | 0.08 |
| Python | 19.22 | 19.19 | 107.2 | - | - |

Toolchains: `sisuc -O2` (LLVM 22), Ubuntu clang 18.1.3 with `-O2`, rustc
1.99.0 with `-C opt-level=3`, Go 1.27.2, OpenJDK 25.0.4.1 (Temurin), Python
3.14.5.

## The programs

| Benchmark | What it does | What it measures |
| --- | --- | --- |
| [fib](../bench/fib) | Computes `fib(40)` by naive recursion | Function calls and integer arithmetic |
| [sieve](../bench/sieve) | Counts the primes up to 50 000 000 with a sieve of Eratosthenes | Array indexing in a tight loop |
| [binary_trees](../bench/binary_trees) | The [binary-trees](https://benchmarksgame-team.pages.debian.net/benchmarksgame/description/binarytrees.html) program of the Benchmarks Game, depth 18 | Allocating and freeing many small objects |

Benchmarks that need floats, strings or input come with the milestones that
add them.

## What the numbers mean

- **Wall** is elapsed time. **CPU** is user plus system time, so the work of a
  garbage collector or JIT thread counts too. Both come from `/usr/bin/time`.
- **Peak RSS** is the most memory the process held at once.
- **Binary** is the size of the stripped executable. Java and Python have none:
  they need their runtime installed. A Sisu binary links the runtime
  (`libsisu_runtime.a`, written in Rust) statically, which accounts for most of
  its 317 KiB.
- **Build** is the time to compile the one file. Go's 1.88 s for fib is a cold
  build cache; later Go builds reuse it.
- Every process runs whole, so startup counts. Java pays for JVM startup and
  JIT warm-up, Python for interpreter startup.
- `taskset` pins each run to one CPU. Go and the JVM therefore see a single
  CPU, and the JVM picks its serial garbage collector.
- The checks differ. Sisu checks both integer overflow and array bounds. Rust,
  Go and Java check bounds but not overflow; C checks neither.
- The inputs are constants in the source. No compiler folded a program to its
  answer: the run times grow with the input.

Java is the fastest on binary_trees, because its collector allocates by
bumping a pointer and frees short-lived objects in bulk. It pays for that with
ten times the memory.

## Where binary_trees spends its time

Sisu takes 1.4 times as long as C on binary_trees. Callgrind counted the
instructions in each function, and each suspect was then removed by hand from
the program's IR (`sisuc -O2 --emit ir`) and the result timed.

- **The allocator costs the same.** `malloc` and `free` run 12.4 billion
  instructions in both programs. A Sisu node is 24 bytes, its count and two
  fields, against C's 16, but glibc rounds both up to a 32-byte chunk, so peak
  memory is the same too.
- **A locked live count, now gone.** The runtime counted live objects for
  `--rc-checks` with an atomic add and subtract. On x86-64 each is a locked
  instruction and a full memory barrier, paid on every allocation and free.
  That was a quarter of the run: Sisu took 1.10 s before the runtime switched
  to a plain load and store.
- **`self` is passed owned.** Before `self.left?.check()` the caller adds 1
  to the child's count, and `check` subtracts 1 from `self` when it returns.
  No count reaches 0 there, but each call writes two nodes and calls
  `sisu_rc.release.Node`. With `self` borrowed instead, the run took 0.75 s
  instead of 0.84 s. Skipping the add is safe here because `left` and `right` are not `mut`:
  the node holds its children for as long as the caller holds the node.
  [ADR 0012](adr/0012-methods-borrow-self.md) proposes this convention.
- **`?.` and `??` build an optional.** `self.left?.check() ?? 0` makes an
  `i64?` and then picks from it, where C branches once. With the counting
  removed, `check` still runs 1.8 times C's instructions, but they are
  register operations that the CPU predicts well.
- **The runtime wraps `malloc`, at no measurable cost.** `sisu_alloc`
  validates the layout and calls Rust's allocator, which calls `malloc`: about
  34 instructions a call. A wrapper that calls `malloc` directly ran no faster,
  so link-time optimization with the runtime (roadmap) would gain little here.

What remains is the counting itself: a store when an object is made, and a
load, subtract and store when it is released. ADR 0005 accepts that cost.

## Running them

```sh
scripts/bench.sh                 # all benchmarks
scripts/bench.sh fib sieve       # some of them
RUNS=10 BENCH_CPU=4 scripts/bench.sh
```

The script builds `sisuc` in release mode and then runs each program in every
language whose toolchain is on `PATH`. It stops if a program prints something
other than the Sisu version, and it prints the tables above with the commit,
date and machine. It needs `clang`, `rustc`, `go`, `javac` and `java`, and
`python3.14` (or the interpreter `PYTHON` names); any that is missing is
skipped. Build outputs go to `target/bench/`.

The numbers depend on the machine. Rerun the script and replace the tables when
a change to `sisuc` or the runtime should move them.

To add a benchmark, add a directory under `bench/` with `main.sisu` and, for
each language, `main.c`, `main.rs`, `main.go`, `Main.java` and `main.py`, all
printing the same output. Then add its name to the default list in
`scripts/bench.sh`.

CodSpeed tracks Sisu against itself on every pull request that touches the
code; see [Coverage and performance](development.md#coverage-and-performance).
