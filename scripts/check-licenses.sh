#!/usr/bin/env bash
# Fails when anything that ships inside the binary has a license outside the list AGPL-3.0 accepts here: Rust crates
# reachable through normal dependencies (not dev, not build) and front-end packages that are not dev-only.
# Also checks that the project itself is AGPL-3.0-only.
# Usage: scripts/check-licenses.sh   (ALLOW overrides the list of SPDX identifiers)
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
# The binary ships for both image platforms; each may pull in dependencies of its own.
TARGETS="x86_64-unknown-linux-musl aarch64-unknown-linux-musl"
allow=${ALLOW:-"MIT Apache-2.0 ISC BSD-2-Clause BSD-3-Clause Unicode-3.0 Zlib CDLA-Permissive-2.0 MIT-0"}
failed=0

# True when the SPDX expression holds with only allowed licenses: OR needs one side, AND needs both.
# "WITH LLVM-exception" narrows nothing that matters here; the old "A/B" form means OR.
spdx_ok() {
  python3 -I -c '
import re, sys
allow = set(sys.argv[2].split())
expr = sys.argv[1].replace(" WITH LLVM-exception", "").replace("/", " OR ")
tokens = re.findall(r"\(|\)|[A-Za-z0-9.+-]+", expr)
py = " ".join({"(": "(", ")": ")", "OR": "or", "AND": "and"}.get(t, str(t in allow)) for t in tokens)
sys.exit(0 if eval(py, {"__builtins__": {}}) else 1)
' "$1" "$allow"
}

# name version license, one per line
{
  for target in $TARGETS; do
  cargo metadata --format-version 1 --locked --filter-platform "$target" | jq -r '
    (.packages | map({key: .id, value: .}) | from_entries) as $pkg
    | (.resolve.nodes | map({key: .id, value: [.deps[] | select(any(.dep_kinds[]; .kind == null)) | .pkg]})
       | from_entries) as $deps
    | [.resolve.root] as $start
    | {seen: $start, todo: $start}
    | until(.todo == []; .todo[0] as $id | .todo |= .[1:]
        | reduce $deps[$id][] as $d (.;
            if (.seen | index($d)) then . else .seen += [$d] | .todo += [$d] end))
    | .seen[] | select(. != $start[0]) | $pkg[.] | "\(.name) \(.version) \(.license // "")"'
  done
  jq -r '.packages | to_entries[] | select(.key != "" and (.value.dev | not))
    | "\(.key | sub("^.*node_modules/"; "")) \(.value.version) \(.value.license // "")"' web/package-lock.json
} | sort -u > "${TMPDIR:-/tmp}/cairn-licenses.$$"

total=0
outside=0
while read -r name version license; do
  total=$((total + 1))
  bad=""
  if [[ -z $license ]]; then
    bad="no license"
  elif ! spdx_ok "$license"; then
    bad="$license"
  fi
  if [[ -n $bad ]]; then
    echo "FAIL $name $version: $bad"
    outside=$((outside + 1))
  fi
done < "${TMPDIR:-/tmp}/cairn-licenses.$$"
rm -f "${TMPDIR:-/tmp}/cairn-licenses.$$"
echo "licenses: $total packages for $TARGETS, $outside outside the list"
(( outside == 0 )) || failed=1

if ! head -1 LICENSE | grep -q 'GNU AFFERO GENERAL PUBLIC LICENSE' || ! sed -n 2p LICENSE | grep -q 'Version 3, 19 November 2007'; then
  echo "FAIL LICENSE is not the AGPL-3.0 text"
  failed=1
fi
if ! grep -q '^license = "AGPL-3.0-only"$' Cargo.toml; then
  echo "FAIL Cargo.toml license is not AGPL-3.0-only"
  failed=1
fi

exit "$failed"
