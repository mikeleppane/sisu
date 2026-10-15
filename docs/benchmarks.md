# Benchmarks

These numbers compare Sisu with C, Rust, Go, Java and Python on small
single-threaded programs. They show where Sisu stands, not how fast it will
be. Each program uses the same algorithm in every language, written plainly,
with no per-language tuning.

## Results

fib and sieve ran at commit 9cbd44e and binary_trees at 7c8d29e (the change
between them is the runtime's live count), on 2026-10-12. hello ran at
commit 4690caa and quicksort at 3dbda29, on 2026-10-15. All ran on an Intel Core i9-14900K under
Linux 7.0. Each figure is the median of 5 runs, pinned to one CPU. Each table
runs from fastest to slowest wall time, with Sisu in bold.

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

### hello

| Language | Wall (s) | CPU (s) | Peak RSS (MiB) | Binary (KiB) | Build (s) |
| --- | ---: | ---: | ---: | ---: | ---: |
| C | 0.00 | 0.00 | 1.1 | 14 | 0.053 |
| Go | 0.00 | 0.00 | 1.9 | 1483 | 0.037 |
| Python | 0.00 | 0.00 | 10.5 | - | - |
| Rust | 0.00 | 0.00 | 1.9 | 328 | 0.073 |
| **Sisu** | **0.00** | **0.00** | **1.4** | **353** | **0.058** |
| Java | 0.01 | 0.00 | 39.2 | - | 0.196 |

### quicksort

| Language | Wall (s) | CPU (s) | Peak RSS (MiB) | Binary (KiB) | Build (s) |
| --- | ---: | ---: | ---: | ---: | ---: |
| C | 0.53 | 0.52 | 77.3 | 14 | 0.054 |
| Go | 0.55 | 0.55 | 78.3 | 1487 | 0.069 |
| Rust | 0.55 | 0.54 | 77.8 | 330 | 0.152 |
| **Sisu** | **0.62** | **0.61** | **77.6** | **353** | **0.056** |
| Java | 0.64 | 0.63 | 117.6 | - | 0.196 |
| Python | 12.66 | 12.65 | 393.7 | - | - |

Toolchains: `sisuc -O2` (LLVM 22), Ubuntu clang 18.1.3 with `-O2`, rustc
1.99.0 with `-C opt-level=3`, Go 1.27.2, OpenJDK 25.0.4.1 (Temurin), Python
3.14.5.

## The programs

| Benchmark | What it does | What it measures |
| --- | --- | --- |
| [fib](../bench/fib) | Computes `fib(40)` by naive recursion | Function calls and integer arithmetic |
| [sieve](../bench/sieve) | Counts the primes up to 50 000 000 with a sieve of Eratosthenes | Array indexing in a tight loop |
| [binary_trees](../bench/binary_trees) | The [binary-trees](https://benchmarksgame-team.pages.debian.net/benchmarksgame/description/binarytrees.html) program of the Benchmarks Game, depth 18 | Allocating and freeing many small objects |
| [hello](../bench/hello) | Prints `Hello, World!` | The time from source to output: build and startup |
| [quicksort](../bench/quicksort) | Sorts 10 000 000 pseudo-random integers with a textbook quicksort, then prints a checksum that depends on their order | Element writes, and in Sisu whether an array passed and returned is written in place |

Benchmarks that need floats come with milestone 6.

## What the numbers mean

- **Wall** is elapsed time. **CPU** is user plus system time, so the work of a
  garbage collector or JIT thread counts too. Both come from `/usr/bin/time`.
- **Peak RSS** is the most memory the process held at once.
- **Binary** is the size of the stripped executable. Java and Python have none:
  they need their runtime installed. A Sisu binary links the runtime
  (`libsisu_runtime.a`, written in Rust) statically, which accounts for most of
  its size.
- **Build** is the wall time to compile the one file and link it. The hello
  and quicksort tables give the median of 5 builds, to the millisecond. The
  older tables time one build, so Go's 1.88 s for fib is a cold build cache.
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

## quicksort on an array value

A Sisu function cannot change its caller's array (ADR 0007), so `quicksort`
takes the array and returns it, and recurses with
`a = quicksort(a, lo, i - 1)`. Last use hands the array over, so each call
holds the only reference and every element write happens in place. Peak
memory shows it: Sisu holds 77.6 MiB, as C does for its one array of
10 000 000 `i64`.

Each element write checks the index against the bounds and the array's
count against 1, and each addition checks for overflow. To find what each
check costs, each kind was removed by hand from the program's IR at commit
c97a45a (`sisuc --emit ir-raw`), and the result built with the same
`default<O2>` pipeline. hyperfine timed each build, pinned to one CPU, as the
mean of 15 runs:

| Removed from the IR | Mean (ms) | Times C |
| --- | ---: | ---: |
| Nothing | 686 | 1.28 |
| The overflow checks | 705 | 1.32 |
| The count checks | 617 | 1.15 |
| The bounds checks | 624 | 1.17 |
| All three | 541 | 1.01 |
| C, for comparison | 535 | 1.00 |

The checks are the whole gap to C. Some were redundant. In the swap
`a[i] = a[j]` then `a[j] = t`, the second write checked the count and the
length of the array the first write had just made unique. LLVM kept those
checks for two reasons:

- It could not tell that storing an element leaves the count and the length
  alone.
- On the copy path of `sisu_rc.unique`, the old block's count was written
  after the copy's header. LLVM could not prove that the old block was not
  the copy, so it could not prove that the copy's count was still 1.

Codegen now tags the count and the length apart from the elements (TBAA,
type-based alias analysis), and `unique` releases the old block before it
allocates the copy. Either change alone gains nothing; together they remove
those checks. Built with and without them, quicksort ran in 635 ms instead of
707 ms. That is as fast as removing every count check by hand, so proving at
compile time that an array has one holder, as Roc's Morphic analysis does,
would gain nothing more here. Marking `sisu_alloc`'s result `noalias` gained
nothing either.

## Where a hello build spends its time

The Build column of hello is the time from source to a program, and every
program then starts in under a millisecond. hyperfine timed the steps on
commit 4690caa, pinned to one CPU, as the mean of 50 runs after 5 warm-up
runs (`taskset -c 2 hyperfine -N -w 5 -r 50 ...`):

| Command | Mean (ms) |
| --- | ---: |
| `sisuc --check main.sisu` | 1.4 |
| `sisuc -O2 main.sisu main` | 27.6 |
| `cc main.o libsisu_runtime.a -lm -o main`, the link step alone | 21.9 |
| `./main` built by `sisuc` | 0.2 |
| `./main` built by `clang` | 0.2 |
| `python3.14 main.py` | 5.3 |

- Lexing, parsing and checking take 1.4 ms.
- The link through `cc`, which `sisuc` runs as its last step, takes about
  22 of the 28 ms. A faster edit-to-output loop starts there, not in the
  compiler.
- `scripts/bench.sh` does not pin or warm up its builds, so its Build column
  reads higher.

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
