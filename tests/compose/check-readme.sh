#!/usr/bin/env bash
# Checks that the install guide is the tested one: every block after <!-- file: examples/compose/... --> equals
# that file, every command and sentence in lib.sh's README_COMMANDS and README_SENTENCES is in the README, and no
# TODO or "not implemented" line is left in the README or the docs.
# Usage: tests/compose/check-readme.sh [README path] [architecture doc path]
set -uo pipefail

source "$(dirname "$0")/lib.sh"
readme=${1:-$ROOT/README.md}
arch=${2:-$ROOT/docs/architecture.md}
failed=0
fail() { echo "FAIL $*"; failed=1; }

python3 -I - "$readme" "$ROOT" <<'PY' || failed=1
import pathlib, re, sys
text = open(sys.argv[1]).read()
root = pathlib.Path(sys.argv[2])
blocks = re.findall(r"<!-- file: (\S+) -->\n```[a-z]*\n(.*?)```", text, re.S)
if not blocks:
    print("FAIL no quoted example files")
    sys.exit(1)
bad = 0
for path, body in blocks:
    if (root / path).read_text() != body:
        print(f"FAIL {path} differs from its copy in the README")
        bad = 1
sys.exit(bad)
PY

lines=$(cat "$readme")
for c in "${README_COMMANDS[@]}"; do
  grep -qxF -- "$c" <<< "$lines" || fail "command missing from the README: $c"
done
for s in "${README_SENTENCES[@]}"; do
  grep -qF -- "$s" <<< "$lines" || fail "sentence missing from the README: $s"
done
if grep -n '\[TODO' "$readme" "$arch"; then fail "TODO left"; fi
if grep -n -i 'nothing \(here\|below\) is implemented' "$readme" "$arch"; then fail "status line says nothing is implemented"; fi

(( failed == 0 )) && echo "readme: ok"
exit "$failed"
