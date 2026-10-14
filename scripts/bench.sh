#!/usr/bin/env bash
# Builds each benchmark in bench/ in every language whose toolchain is on PATH,
# checks that it prints what the Sisu version prints, and times it pinned to
# one CPU. Prints a markdown table for docs/benchmarks.md.
#   scripts/bench.sh [benchmark...]
# RUNS (default 5) sets the runs per program, BENCH_CPU (default 2) the CPU,
# PYTHON (default python3.14) the Python interpreter.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
runs=${RUNS:-5}
cpu=${BENCH_CPU:-2}
python=${PYTHON:-python3.14}
out=target/bench
benchmarks=("$@")
[[ ${#benchmarks[@]} -gt 0 ]] || benchmarks=(fib sieve binary_trees wc)

cargo build --release --quiet -p sisuc -p sisu-runtime

have() { command -v "$1" >/dev/null; }

# Builds benchmark $2 in language $1 into directory $3, timing the build into
# $3/compile, and prints the command that runs it.
build() {
    local lang=$1 src=bench/$2 dir=$3
    local t=(/usr/bin/time -f %e -o "$dir/compile")
    case $lang in
        sisu) "${t[@]}" target/release/sisuc -O2 "$src/main.sisu" "$dir/main" ;;
        c) "${t[@]}" clang -O2 -o "$dir/main" "$src/main.c" ;;
        rust) "${t[@]}" rustc -C opt-level=3 -o "$dir/main" "$src/main.rs" ;;
        go) "${t[@]}" go build -o "$dir/main" "$src/main.go" ;;
        java) "${t[@]}" javac -d "$dir" "$src/Main.java" && echo "java -cp $dir Main" && return ;;
        python) echo "$python $src/main.py" && return ;;
    esac
    echo "$dir/main"
}

# The median of the numbers on standard input.
median() { sort -n | awk '{ v[NR] = $1 } END { print v[int((NR + 1) / 2)] }'; }

declare -A names=([sisu]=Sisu [c]=C [rust]=Rust [go]=Go [java]=Java [python]=Python)
available=(sisu)
have clang && available+=(c)
have rustc && available+=(rust)
have go && available+=(go)
have javac && have java && available+=(java)
have "$python" && available+=(python)

echo "Commit $(git rev-parse --short HEAD), $(date -u +%F), $(lscpu | sed -n 's/^Model name: *//p'), $(uname -sr)."
echo "Median of $runs runs on CPU $cpu."
for bench in "${benchmarks[@]}"; do
    rows=()
    # A benchmark with an input.awk reads what it writes on standard input.
    input=/dev/null
    if [[ -f bench/$bench/input.awk ]]; then
        input=$out/$bench/input
        mkdir -p "$out/$bench"
        [[ $input -nt bench/$bench/input.awk ]] || awk -f "bench/$bench/input.awk" >"$input"
    fi
    for lang in "${available[@]}"; do
        dir=$out/$bench/$lang
        rm -rf "$dir" && mkdir -p "$dir"
        cmd=$(build "$lang" "$bench" "$dir")
        for i in $(seq "$runs"); do
            # shellcheck disable=SC2086 # $cmd is a command and its arguments.
            taskset -c "$cpu" /usr/bin/time -f "%e %U %S %M" -o "$dir/time.$i" $cmd <"$input" >"$dir/stdout"
            if ! cmp -s "$dir/stdout" "$out/$bench/sisu/stdout"; then
                echo "$bench/$lang printed something other than the Sisu version:" >&2
                diff "$out/$bench/sisu/stdout" "$dir/stdout" >&2
                exit 1
            fi
        done
        wall=$(cat "$dir"/time.* | awk '{ print $1 }' | median)
        cpu_s=$(cat "$dir"/time.* | awk '{ printf "%.2f\n", $2 + $3 }' | median)
        rss=$(cat "$dir"/time.* | awk '{ printf "%.1f\n", $4 / 1024 }' | median)
        binary=-
        if [[ -f $dir/main ]]; then
            strip -o "$dir/main.stripped" "$dir/main"
            binary=$(($(stat -c %s "$dir/main.stripped") / 1024))
        fi
        compile=-
        [[ -f $dir/compile ]] && compile=$(cat "$dir/compile")
        cells=("${names[$lang]}" "$wall" "$cpu_s" "$rss" "$binary" "$compile")
        if [[ $lang == sisu ]]; then
            for i in "${!cells[@]}"; do
                [[ ${cells[i]} == - ]] || cells[i]="**${cells[i]}**"
            done
        fi
        printf -v row '%s | ' "${cells[@]}"
        # The wall time before the tab is the sort key, cut off before printing.
        rows+=("$wall	| ${row% }")
    done
    echo
    echo "### $bench"
    echo
    echo "| Language | Wall (s) | CPU (s) | Peak RSS (MiB) | Binary (KiB) | Build (s) |"
    echo "| --- | ---: | ---: | ---: | ---: | ---: |"
    printf '%s\n' "${rows[@]}" | sort -n | cut -f2-
done

echo
echo "Toolchains:"
echo "- sisuc -O2 (LLVM 22)"
have clang && echo "- $(clang --version | head -1), -O2"
have rustc && echo "- $(rustc --version), -C opt-level=3"
have go && echo "- $(go version)"
have java && echo "- $(java -version 2>&1 | head -1)"
have "$python" && echo "- $($python --version)"
exit 0
