#!/usr/bin/env bash
# Fuzzes the compiler front end (fuzz/fuzz_targets/front_end.rs) with cargo-fuzz.
# Every libFuzzer flag lives here, so CI never repeats one.
#   scripts/fuzz.sh [seconds]          fuzz for <seconds>, 1800 by default
#   scripts/fuzz.sh --replay <input>   run one input
#   scripts/fuzz.sh --tmin <input>     minimize one input into fuzz/artifacts/front_end/
#   scripts/fuzz.sh --cmin             minimize fuzz/corpus/front_end in place
# A fuzz run writes fuzz/artifacts/fuzz.log, run.txt and summary.md, and failing inputs
# land in fuzz/artifacts/front_end/. It exits 1 when it produced a crash-, timeout- or
# oom- artifact, and otherwise with cargo-fuzz's status.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
export RUST_BACKTRACE=1
limits=(-max_len=16384 -timeout=60 -rss_limit_mb=2048)
corpus=fuzz/corpus/front_end
found=fuzz/artifacts/front_end

case ${1:-} in
--replay) exec cargo fuzz run --sanitizer none front_end "${2:?--replay needs an input}" -- "${limits[@]}" ;;
--tmin) exec cargo fuzz tmin --sanitizer none front_end "${2:?--tmin needs an input}" -- "${limits[@]}" ;;
--cmin) exec cargo fuzz cmin --sanitizer none front_end "$corpus" -- "${limits[@]}" ;;
esac

seconds=${1:-1800}
if [[ ! $seconds =~ ^[0-9]+$ ]]; then
    echo "usage: scripts/fuzz.sh [seconds] | --replay <input> | --tmin <input> | --cmin" >&2
    exit 2
fi

# libFuzzer exits 1 when an explicit corpus directory is missing, as on a first run.
mkdir -p "$corpus" "$found"

seeds=$(mktemp -d)
marker=$(mktemp)
trap 'rm -rf "$seeds" "$marker"' EXIT
i=0
while IFS= read -r -d '' f; do
    i=$((i + 1))
    cp "$f" "$seeds/$i-${f##*/}"
done < <(find crates/sisuc/tests -name '*.sisu' -print0)

# Artifacts are named by the input's hash, so an old file can be rewritten: only those
# newer than this marker belong to this run.
touch "$marker"

cargo fuzz build --sanitizer none front_end
cmd=(cargo fuzz run --sanitizer none --jobs "$(nproc)" front_end "$corpus" "$seeds"
    -- -dict="$PWD/fuzz/front_end.dict" -max_total_time="$seconds" "${limits[@]}")
start=$SECONDS
status=0
"${cmd[@]}" 2>&1 | tee fuzz/artifacts/fuzz.log || status=$?
elapsed=$((SECONDS - start))

{
    echo "commit: $(git rev-parse HEAD)"
    if [[ -n $(git status --porcelain) ]]; then echo "tree: dirty"; else echo "tree: clean"; fi
    rustc -V
    cargo fuzz --version
    echo "command: ${cmd[*]}"
} >fuzz/artifacts/run.txt

failures=()
while IFS= read -r -d '' f; do
    case ${f##*/} in
    crash-*) failures+=("crash $f") ;;
    timeout-*) failures+=("timeout $f") ;;
    oom-*) failures+=("oom $f") ;;
    esac
done < <(find "$found" -type f -newer "$marker" -print0 | sort -z)

# Fork mode interleaves the children's output, whose lines start `#<n>\t`; the parent's
# status lines start `#<n>: cov:`, and the last one holds the total.
executions=$(awk '/^#[0-9]+: cov:/ { n = substr($1, 2, length($1) - 2) } END { print (n == "" ? "unknown" : n) }' fuzz/artifacts/fuzz.log)

{
    echo "# Fuzzing: front_end"
    echo
    echo "- Fuzzed for ${elapsed} s (asked for ${seconds} s)."
    echo "- Executions in processed jobs: ${executions}."
    echo "- cargo-fuzz exit status: ${status}."
    echo
    echo "## Failures"
    echo
    if ((${#failures[@]} == 0)); then
        echo "None."
    fi
    for failure in "${failures[@]}"; do
        class=${failure%% *}
        path=${failure#* }
        echo "- **${class}**: \`${path}\`. Replay: \`scripts/fuzz.sh --replay ${path}\`."
        if [[ $class != crash ]]; then
            echo "  No Rust backtrace: without a sanitizer, libFuzzer prints no native stack for this class."
        fi
    done
} >fuzz/artifacts/summary.md

cat fuzz/artifacts/summary.md
if ((${#failures[@]} > 0)); then
    exit 1
fi
exit "$status"
