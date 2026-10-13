#!/usr/bin/env bash
# Fails when a unit test reads a diagnostic's wording: its message, its label,
# or its help text beyond whether there is one. Unit tests assert codes and
# positions; the tests/diagnostics fixtures pin the wording (CODING_STANDARDS.md,
# Tests). The writers' own tests in render.rs, json.rs and report.rs may read it.
# Test code is files under tests/, tests.rs, and everything after a file's
# first top-level #[cfg(test)].
#   scripts/check-test-wording.sh <file.rs>...
set -euo pipefail

files=()
for file in "$@"; do
  [[ $file == */diagnostic/render.rs || $file == */diagnostic/json.rs || $file == */src/report.rs ]] || files+=("$file")
done
[[ ${#files[@]} -eq 0 ]] && exit 0

perl -0777 -ne '
  pos = $ARGV =~ m{/tests/|/tests\.rs$} ? 0 : /^#\[cfg\(test\)\]/m ? $-[0] : length;
  while (/\.(?:message|label)\(\)|\.help\(\)(?!\.is_(?:some|none)\(\)|,\s*None\b)/g) {
    my $line = 1 + (substr($_, 0, $-[0]) =~ tr/\n//);
    print "$ARGV:$line: a unit test reads diagnostic wording; assert the code, and pin the text in a tests/diagnostics fixture\n";
    $bad = 1;
  }
  END { exit($bad ? 1 : 0) }
' "${files[@]}"
