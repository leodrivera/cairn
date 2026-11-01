#!/usr/bin/env bash
# Checks that the install guide is the tested one: every block after <!-- file: examples/compose/... --> in the
# README or docs/install.md equals that file, every command and sentence in lib.sh's README_COMMANDS and
# README_SENTENCES is in one of the two, and no TODO or "not implemented" line is left in them or the docs.
# Usage: tests/compose/check-readme.sh [README path] [architecture doc path] [install guide path]
set -uo pipefail

source "$(dirname "$0")/lib.sh"
readme=${1:-$ROOT/README.md}
arch=${2:-$ROOT/docs/architecture.md}
guide=${3:-$ROOT/docs/install.md}
failed=0
fail() { echo "FAIL $*"; failed=1; }

[[ -f $guide ]] || { echo "FAIL no install guide at $guide"; exit 1; }

python3 -I - "$readme" "$guide" "$ROOT" <<'PY' || failed=1
import pathlib, re, sys
text = open(sys.argv[1]).read() + open(sys.argv[2]).read()
root = pathlib.Path(sys.argv[3])
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

lines=$(cat "$readme" "$guide")
for c in "${README_COMMANDS[@]}"; do
  grep -qxF -- "$c" <<< "$lines" || fail "command missing from the README and the install guide: $c"
done
for s in "${README_SENTENCES[@]}"; do
  grep -qF -- "$s" <<< "$lines" || fail "sentence missing from the README and the install guide: $s"
done
if grep -n '\[TODO' "$readme" "$guide" "$arch"; then fail "TODO left"; fi
# The image name is the real one everywhere a reader copies it from.
if grep -n 'ghcr\.io/OWNER/' "$readme" "$guide" "$arch" "$ROOT"/examples/compose/*/compose.yml; then
  fail "the OWNER placeholder is left in place of the image's owner"
fi
if grep -n -i 'nothing \(here\|below\) is implemented' "$readme" "$guide" "$arch"; then fail "status line says nothing is implemented"; fi

(( failed == 0 )) && echo "readme: ok"
exit "$failed"
