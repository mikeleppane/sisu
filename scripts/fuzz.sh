#!/usr/bin/env bash
# Fuzzes the compiler front end (fuzz/fuzz_targets/front_end.rs) with cargo-fuzz.
# Every libFuzzer flag lives here, so CI never repeats one.
#   scripts/fuzz.sh [seconds]          fuzz for <seconds>, 1800 by default
#   scripts/fuzz.sh --replay <input>   run one input
#   scripts/fuzz.sh --tmin <input>     minimize one input into fuzz/artifacts/front_end/
#   scripts/fuzz.sh --cmin             minimize fuzz/corpus/front_end in place
# A fuzz run writes fuzz/artifacts/fuzz.log, run.txt and summary.md, and failing inputs
# land in fuzz/artifacts/front_end/. Each new crash- input is then replayed once, and its
# output appended to fuzz.log. The run exits 1 when it produced a crash-, timeout- or
# oom- artifact, and otherwise with cargo-fuzz's status.
set -euo pipefail

# Resolve the input before the cd, so a path relative to the caller's directory works.
case ${1:-} in
--replay | --tmin) input=$(realpath "${2:?$1 needs an input}") ;;
esac
cd "$(git rev-parse --show-toplevel)"
export RUST_BACKTRACE=1
limits=(-max_len=16384 -timeout=60 -rss_limit_mb=2048)
# Seconds for the replay of one crash, past libFuzzer's own -timeout, and how many crashes
# are replayed: a regression can leave one crash- per corpus input, and the job has a limit.
replay_seconds=120
replay_limit=3
corpus=fuzz/corpus/front_end
found=fuzz/artifacts/front_end

case ${1:-} in
--replay) exec cargo fuzz run --sanitizer none front_end "$input" -- "${limits[@]}" ;;
--tmin) exec cargo fuzz tmin --sanitizer none front_end "$input" -- "${limits[@]}" ;;
--cmin) exec cargo fuzz cmin --sanitizer none front_end "$corpus" -- "${limits[@]}" ;;
esac

seconds=${1:-1800}
if [[ ! $seconds =~ ^[0-9]+$ ]]; then
    echo "usage: scripts/fuzz.sh [seconds] | --replay <input> | --tmin <input> | --cmin" >&2
    exit 2
fi

# libFuzzer exits 1 when an explicit corpus directory is missing, as on a first run.
mkdir -p "$corpus" "$found"
log=fuzz/artifacts/fuzz.log
: >"$log"

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

# Runs a command with its output teed into the log. Under pipefail `|| status=$?` would
# take tee's status, so status comes from PIPESTATUS; a failed tee only marks the log.
status=0
log_complete=true
logged() {
    local pipe=(0 0)
    "$@" 2>&1 | tee -a "$log" || pipe=("${PIPESTATUS[@]}")
    status=${pipe[0]}
    if ((pipe[1] != 0)); then log_complete=false; fi
}

# cargo-fuzz adds -fork=N only when N != 1, so a one-CPU machine still gets 2 jobs.
jobs=$(nproc)
if ((jobs < 2)); then jobs=2; fi
cmd=(cargo fuzz run --sanitizer none --jobs "$jobs" front_end "$corpus" "$seeds"
    -- -dict="$PWD/fuzz/front_end.dict" -max_total_time="$seconds" "${limits[@]}")
logged cargo fuzz build --sanitizer none front_end
start=$SECONDS
if ((status == 0)); then logged "${cmd[@]}"; fi
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

# Fork mode's first pass over the corpus sends the children's output to /dev/null, so a
# crash on a restored input leaves no panic message in the log. A plain replay prints it;
# its own status changes neither the run's status nor which inputs count.
crashes=0
for failure in "${failures[@]}"; do
    if [[ ${failure%% *} != crash ]]; then continue; fi
    crashes=$((crashes + 1))
    if ((crashes > replay_limit)); then continue; fi
    path=${failure#* }
    {
        echo
        echo "==> scripts/fuzz.sh: replay of ${path}"
        replay=0
        timeout -k 10 "$replay_seconds" \
            cargo fuzz run --sanitizer none front_end "$path" -- "${limits[@]}" || replay=$?
        echo "==> scripts/fuzz.sh: the replay exited ${replay}"
    } >>"$log" 2>&1 || log_complete=false
done
not_replayed=0
if ((crashes > replay_limit)); then
    not_replayed=$((crashes - replay_limit))
    echo "==> scripts/fuzz.sh: ${not_replayed} more crashes not replayed; scripts/fuzz.sh --replay <path> replays any of them" >>"$log" || log_complete=false
fi

# Fork mode interleaves the children's output, whose lines start `#<n>\t`; the parent's
# status lines start `#<n>: cov:`, and the last one holds the total.
executions=$(awk '/^#[0-9]+: cov:/ { n = substr($1, 2, length($1) - 2) } END { print (n == "" ? "unknown" : n) }' "$log")

{
    echo "# Fuzzing: front_end"
    echo
    echo "- Fuzzed for ${elapsed} s (asked for ${seconds} s)."
    echo "- Executions in processed jobs: ${executions}."
    echo "- cargo-fuzz exit status: ${status}."
    if [[ $log_complete == false ]]; then
        echo "- \`fuzz.log\` is incomplete: writing it failed."
    fi
    echo
    echo "## Failures"
    echo
    if ((${#failures[@]} == 0 && status != 0)); then
        echo "The fuzzer exited ${status} with no failing input; see \`fuzz.log\`."
    elif ((${#failures[@]} == 0)); then
        echo "None."
    else
        echo "Paths are the run's own. Replay a downloaded input by its own path: \`--replay\` resolves it from your directory."
        echo
    fi
    for failure in "${failures[@]}"; do
        class=${failure%% *}
        path=${failure#* }
        echo "- **${class}**: \`${path}\`. Replay: \`scripts/fuzz.sh --replay ${path}\`."
        if [[ $class == crash ]]; then
            echo "  Its replay is at the end of \`fuzz.log\`. A crash whose replay shows no panic message is a stack overflow or another fatal signal."
        else
            echo "  No Rust backtrace: without a sanitizer, libFuzzer prints no native stack for this class."
        fi
    done
    if ((not_replayed > 0)); then
        echo
        echo "${not_replayed} more crashes were not replayed. \`scripts/fuzz.sh --replay <path>\` replays any of them."
    fi
} >fuzz/artifacts/summary.md

cat fuzz/artifacts/summary.md
if ((${#failures[@]} > 0)); then
    exit 1
fi
exit "$status"
