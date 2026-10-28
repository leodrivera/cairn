#!/usr/bin/env bash
# Fails when the image breaks Cairn's budget: binary size, idle memory, front-end size.
# Usage: scripts/check-budget.sh <image> [web-dist-folder]
# Limits use binary units (1 MB = 1048576 bytes, 1 kB = 1024 bytes). Override with
# BUDGET_BINARY_MB, BUDGET_MEMORY_MB and BUDGET_WEB_KB.
set -euo pipefail

image=${1:?usage: scripts/check-budget.sh <image> [web-dist-folder]}
dist=${2:-web/dist}
binary_mb=${BUDGET_BINARY_MB:-12}
memory_mb=${BUDGET_MEMORY_MB:-20}
web_kb=${BUDGET_WEB_KB:-100}

work=$(mktemp -d)
name="cairn-budget-$$"
cleanup() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

failed=0
report() { # label bytes limit-bytes unit-bytes unit
  local value limit
  value=$(awk -v b="$2" -v u="$4" 'BEGIN { printf "%.1f", b / u }')
  limit=$(awk -v b="$3" -v u="$4" 'BEGIN { printf "%g", b / u }')
  echo "$1 $value $5 (limit $limit $5)"
  if (( $2 > $3 )); then failed=1; fi
}

# Binary size
docker create --name "$name" "$image" >/dev/null
docker cp "$name:/cairn" "$work/cairn" >/dev/null
docker rm "$name" >/dev/null
report binary "$(stat -c %s "$work/cairn")" $((binary_mb * 1048576)) 1048576 MB

# Idle memory, 5 s after start
docker run -d --name "$name" "$image" >/dev/null
sleep 5
if [[ $(docker inspect -f '{{.State.Running}}' "$name") != true ]] || ! docker exec "$name" /cairn health; then
  echo "memory: the container is not running or not healthy; its log:"
  docker logs "$name" 2>&1 | tail -20
  exit 1
fi
usage=$(docker stats --no-stream --format '{{.MemUsage}}' "$name" | awk '{ print $1 }')
bytes=$(awk -v u="$usage" 'BEGIN {
  n = u + 0; s = u; sub(/^[0-9.]+/, "", s)
  m["B"] = 1; m["KiB"] = 1024; m["MiB"] = 1048576; m["GiB"] = 1073741824
  m["kB"] = 1000; m["MB"] = 1000000; m["GB"] = 1000000000
  if (!(s in m)) { print "unknown unit " s > "/dev/stderr"; exit 1 }
  printf "%d", n * m[s]
}')
report memory "$bytes" $((memory_mb * 1048576)) 1048576 MB

# Front-end, gzipped
web=$(cat "$dist"/assets/* | gzip -9 | wc -c)
report "front-end" "$web" $((web_kb * 1024)) 1024 "kB gzipped"

exit "$failed"
