#!/usr/bin/env bash
# Runs the README's Fail2ban filter over fixed log lines: Cairn's failed sign-ins in both forms (as written, and in
# Docker's json-file log) must count with the client's address, and no line may let a typed value, another event or
# another container's log choose the address. Needs Docker.
set -uo pipefail

source "$(dirname "$0")/lib.sh"
FAIL2BAN=crazymax/fail2ban@sha256:2bc26b68f373ce6e52bbd181de2937a6694f72b74ddf15bbd1b25e90553dcda3  # 1.1.0
want=$'203.0.113.10\n203.0.113.7\n203.0.113.8'
got=$(docker run --rm -v "$TESTS/fail2ban/lines.log:/lines.log:ro" \
  -v "$ROOT/examples/compose/fail2ban/cairn.conf:/filter.conf:ro" \
  --entrypoint fail2ban-regex "$FAIL2BAN" -o ip /lines.log /filter.conf 2>&1 | sort)
if [[ $got == "$want" ]]; then
  echo "fail2ban: ok"
else
  echo "FAIL fail2ban filter counted: $(tr '\n' ' ' <<< "$got"), want: $(tr '\n' ' ' <<< "$want")"
  exit 1
fi
