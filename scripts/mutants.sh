#!/usr/bin/env bash
# Mutation-tests the changes since <base>: by default where HEAD left main.
# On a stacked branch, pass the stage's base so earlier stages stay out.
# Arguments after <base> go to cargo mutants, for example --list or -j 8.
# Runs 4 mutants at a time unless -j or CARGO_MUTANTS_JOBS says otherwise:
# measured, -j 4 beats serial and -j 8 gains nothing (nextest fills the cores).
#   scripts/mutants.sh [base] [cargo-mutants args...]
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
base=${1:-$(git merge-base main HEAD)}
[[ $# -gt 0 ]] && shift

diff=$(mktemp)
trap 'rm -f "$diff"' EXIT
git diff "$base" >"$diff"

# .cargo/mutants.toml leaves .llvm/ out of the copied tree, so point the
# build at the original.
CARGO_MUTANTS_JOBS=${CARGO_MUTANTS_JOBS:-4} \
  LLVM_SYS_221_PREFIX="$PWD/.llvm/22" cargo mutants --in-diff "$diff" "$@"
