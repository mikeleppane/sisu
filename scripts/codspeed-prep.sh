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
sections=200

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
    target/release/sisuc -O2 "$out/$b/main.sisu" "$out/$b/main" || fail "$b failed to compile"
}
reduce fib 's/fib(40)/fib(27)/'
reduce sieve 's/sieve(50_000_000)/sieve(1_000_000)/'
reduce binary_trees 's/max_depth = 18/max_depth = 10/'

# step0 returns its argument; stepi adds 1 through an array, a class and an
# optional chain, then calls step(i-1). main prints step(N-1)(0) = N-1. Not fi:
# f64 is a built-in type.
{
    echo 'fn step0(x: i64) -> i64 { x }'
    for ((i = 1; i < sections; i++)); do
        cat <<SISU

class C$i {
    v: i64

    fn get(self) -> i64 { self.v }
}

fn step$i(x: i64) -> i64 {
    let xs = [x, $i, 1]
    let mut sum = 0
    for v in xs { sum += v }
    let c: C$i? = C$i(v: sum - x - xs[1])
    step$((i - 1))(x + (c?.get() ?? 0))
}
SISU
    done
    printf '\nfn main() {\n    print(step%d(0))\n}\n' $((sections - 1))
} >"$out/large/main.sisu"

# Reads codspeed.yml strictly: a "benchmarks:" line, then per entry the lines
# "  - name: X", "    id: X" (same X) and "    exec: plain words", in that
# order. Anything else fails, so no entry can escape the check below.
names=()
execs=()
state=start
while IFS= read -r line; do
    [[ -z $line || $line == \#* ]] && continue
    case $state:$line in
        start:"benchmarks:") state=name ;;
        name:"  - name: "?* | id:"  - name: "?*)
            [[ $state == name ]] || fail "codspeed.yml: entry lacks id/exec before: $line"
            names+=("${line#  - name: }")
            state=id
            ;;
        id:"    id: "?*)
            [[ ${line#    id: } == "${names[-1]}" ]] || fail "codspeed.yml: id differs from name: $line"
            state=run
            ;;
        run:"    exec: "?*)
            e=${line#    exec: }
            [[ $e != *[\"\'\\#]* ]] || fail "codspeed.yml: exec must be plain words: $line"
            execs+=("$e")
            state=name
            ;;
        *) fail "codspeed.yml: unexpected line: $line" ;;
    esac
done <codspeed.yml
[[ $state == name && ${#names[@]} -gt 0 ]] || fail "codspeed.yml: no complete benchmarks found"

# Runs the exec line of every entry once. check_* and build_* must print
# nothing; a program must print its expected output and nothing on stderr.
expected() {
    case $1 in
        fib) echo 196418 ;;
        sieve) echo 78498 ;;
        binary_trees) printf '%s\n' 4095 1024 31744 256 32512 64 32704 16 32752 2047 ;;
        *) return 1 ;;
    esac
}
for i in "${!names[@]}"; do
    name=${names[i]}
    read -ra cmd <<<"${execs[i]}"
    "${cmd[@]}" >"$out/stdout" 2>"$out/stderr" || fail "$name exited non-zero"
    [[ ! -s $out/stderr ]] || fail "$name printed to stderr"
    case $name in
        check_* | build_*)
            [[ ! -s $out/stdout ]] || fail "$name printed output"
            ;;
        *)
            expected "$name" >"$out/expected" || fail "no expected output for $name"
            cmp -s "$out/expected" "$out/stdout" || fail "$name printed something other than its expected output"
            ;;
    esac
done
"$out/build_large/main" >"$out/stdout" 2>"$out/stderr" || fail "build_large's program exited non-zero"
[[ ! -s $out/stderr ]] || fail "build_large's program printed to stderr"
printf '%d\n' $((sections - 1)) | cmp -s - "$out/stdout" || fail "build_large's program printed something other than $((sections - 1))"
echo "codspeed-prep: ok"
