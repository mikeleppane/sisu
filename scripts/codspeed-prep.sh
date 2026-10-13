#!/usr/bin/env bash
# Prepares the CodSpeed benchmarks listed in codspeed.yml: builds release sisuc,
# compiles reduced copies of the bench/ programs, generates a large compiler
# input, then runs every benchmark once and checks what it prints. Exits
# non-zero, naming the benchmark, when a reduction no longer applies or a
# benchmark misbehaves.
#   scripts/codspeed-prep.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
out=target/codspeed
sections=1000

cargo build --release --locked -p sisuc -p sisu-runtime
rm -rf "$out"
mkdir -p "$out"/{fib,sieve,binary_trees,large,build_binary_trees,build_large}

fail() {
    echo "codspeed-prep: $*" >&2
    exit 1
}

# Copies bench/$1/main.sisu with sed script $2 applied and compiles it.
reduce() {
    local b=$1 script=$2
    sed "$script" "bench/$b/main.sisu" >"$out/$b/main.sisu"
    if cmp -s "bench/$b/main.sisu" "$out/$b/main.sisu"; then
        fail "the reduction of $b no longer applies"
    fi
    target/release/sisuc -O2 "$out/$b/main.sisu" "$out/$b/main"
}
reduce fib 's/fib(40)/fib(27)/'
reduce sieve 's/sieve(50_000_000)/sieve(1_000_000)/'
reduce binary_trees 's/max_depth = 18/max_depth = 10/'

# f0 returns its argument; fi adds 1 through an array, a class and an optional
# chain, then calls f(i-1). main prints f(N-1)(0) = N-1.
{
    echo 'fn f0(x: i64) -> i64 { x }'
    for ((i = 1; i < sections; i++)); do
        cat <<SISU

class C$i {
    v: i64

    fn get(self) -> i64 { self.v }
}

fn f$i(x: i64) -> i64 {
    let xs = [x, $i, 1]
    let mut sum = 0
    for v in xs { sum += v }
    let c: C$i? = C$i(v: sum - x - xs[1])
    f$((i - 1))(x + (c?.get() ?? 0))
}
SISU
    done
    printf '\nfn main() {\n    print(f%d(0))\n}\n' $((sections - 1))
} >"$out/large/main.sisu"

# Runs the exec line of every codspeed.yml entry once. check_* and build_* must
# print nothing; a program must print what its expected file holds.
expected() {
    case $1 in
        fib) echo 196418 ;;
        sieve) echo 78498 ;;
        binary_trees) printf '%s\n' 4095 1024 31744 256 32512 64 32704 16 32752 2047 ;;
    esac
}
names=()
while IFS= read -r line; do
    [[ $line =~ ^[[:space:]]*-\ name:\ (.+)$ ]] && names+=("${BASH_REMATCH[1]}")
done <codspeed.yml
[[ ${#names[@]} -gt 0 ]] || fail "no benchmarks found in codspeed.yml"
for name in "${names[@]}"; do
    exec_line=$(sed -n "/^  - name: $name\$/,/^  - name:/{s/^    exec: //p}" codspeed.yml)
    [[ -n $exec_line ]] || fail "$name has no exec line in codspeed.yml"
    read -ra cmd <<<"$exec_line"
    "${cmd[@]}" >"$out/stdout" 2>"$out/stderr" || fail "$name exited non-zero"
    case $name in
        check_* | build_*)
            [[ ! -s $out/stdout && ! -s $out/stderr ]] || fail "$name printed output"
            ;;
        *)
            expected "$name" | cmp -s - "$out/stdout" || fail "$name printed something other than its expected output"
            ;;
    esac
done
[[ $("$out/build_large/main") == $((sections - 1)) ]] || fail "build_large printed something other than $((sections - 1))"
echo "codspeed-prep: ok"
