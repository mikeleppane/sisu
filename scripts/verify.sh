#!/usr/bin/env bash
# Runs scripts/coverage.sh, then scripts/mutants.sh, on the changes since <base>,
# and ends with one verdict line that starts "verify: PASS" or "verify: FAIL".
# It stops before the mutants when the coverage run fails or an uncovered region
# touches the changes: the tests that fix that change the mutants' results too.
#   scripts/verify.sh [base]
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
base=${1:-$(git merge-base main HEAD)}

report=$(mktemp)
trap 'rm -f "$report"' EXIT

if ! scripts/coverage.sh "$base" | tee "$report"; then
	echo "verify: FAIL: the coverage run failed; mutants not run"
	exit 1
fi
uncovered=$(sed -n '/^Uncovered regions touching changed code:$/,/^$/p' "$report" |
	sed '1d;/^$/d;/^(none)$/d' | wc -l)
if ((uncovered > 0)); then
	echo "verify: FAIL: $uncovered uncovered regions touch the changes (listed above); mutants not run"
	exit 1
fi

status=0
scripts/mutants.sh "$base" || status=$?
count() { if [[ -f mutants.out/$1.txt ]]; then wc -l <"mutants.out/$1.txt"; else echo 0; fi; }
summary="$(count caught) caught, $(count unviable) unviable, $(count missed) missed, $(count timeout) timeouts"
if ((status == 0)); then
	echo "verify: PASS: no uncovered region touches the changes; mutants: $summary"
else
	echo "verify: FAIL: mutants: $summary (cargo mutants exit $status; see mutants.out/missed.txt)"
	exit 1
fi
