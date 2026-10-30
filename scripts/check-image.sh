#!/usr/bin/env bash
# Checks what the docs promise about the image: scratch contents, a static binary for the image's architecture,
# uid 10001, the HEALTHCHECK, the labels and the version. One "ok" or "FAIL" line per check.
# Usage: scripts/check-image.sh <image>
set -uo pipefail

image=${1:?usage: scripts/check-image.sh <image>}
root=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)
description=$(sed -n 's/^description = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)

work=$(mktemp -d)
name="cairn-check-$$"
cleanup() {
  docker rm -f "$name" "$name-run" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

failed=0
ok() { echo "ok $1"; }
fail() { echo "FAIL $1: $2"; failed=1; }

# Contents: everything the runtime adds to a container (.dockerenv, /dev, /proc, /sys, /etc/hosts and friends) aside.
if docker create --name "$name" "$image" >/dev/null 2>&1; then
  docker export "$name" | tar -tv > "$work/list"
  docker cp "$name:/cairn" "$work/cairn" >/dev/null 2>&1 || true
  docker rm "$name" >/dev/null
  awk '{ print $1 " " $2 " " $NF }' "$work/list" \
    | grep -v -E ' (\.dockerenv|dev/.*|proc/|sys/|etc/(hostname|hosts|resolv\.conf)|/proc/mounts)$' \
    | sed -E 's/^(.)[^ ]* /\1 /' | sort > "$work/got"
  cat > "$work/want" <<'EOF'
- 0/0 cairn
- 0/0 etc/passwd
- 0/0 etc/ssl/certs/ca-certificates.crt
d 0/0 etc/
d 0/0 etc/ssl/
d 0/0 etc/ssl/certs/
d 10001/10001 data/
d 10001/10001 srv/
EOF
  sort -o "$work/want" "$work/want"
  if diff -u "$work/want" "$work/got" > "$work/diff"; then ok contents; else fail contents "$(tr '\n' ' ' < "$work/diff")"; fi
else
  fail contents "cannot create a container from $image"
fi

# Static binary for the image's architecture
arch=$(docker image inspect -f '{{.Architecture}}' "$image" 2>/dev/null)
if [[ -s $work/cairn ]] && readelf -hW "$work/cairn" >/dev/null 2>&1; then
  if readelf -lW "$work/cairn" | grep -q INTERP || readelf -dW "$work/cairn" | grep -q NEEDED; then
    fail static "/cairn has an interpreter or needs shared libraries"
  else
    ok static
  fi
  machine=$(readelf -hW "$work/cairn" | sed -n 's/^ *Machine: *//p')
  case "$arch:$machine" in
    "amd64:Advanced Micro Devices X86-64" | "arm64:AArch64") ok arch ;;
    *) fail arch "image is $arch, /cairn is $machine" ;;
  esac
else
  fail static "no /cairn ELF in $image"
  fail arch "no /cairn ELF in $image"
fi

# Config
user=$(docker image inspect -f '{{.Config.User}}' "$image" 2>/dev/null)
[[ $user == "10001:10001" ]] && ok user || fail user "Config.User is '$user'"

health=$(docker image inspect -f '{{json .Config.Healthcheck.Test}}' "$image" 2>/dev/null)
[[ $health == '["CMD","/cairn","health"]' ]] && ok healthcheck || fail healthcheck "Healthcheck.Test is $health"

label() { docker image inspect -f "{{index .Config.Labels \"$1\"}}" "$image" 2>/dev/null; }
missing=""
[[ $(label org.opencontainers.image.title) == cairn ]] || missing+=" title"
[[ $(label org.opencontainers.image.description) == "$description" ]] || missing+=" description"
[[ $(label org.opencontainers.image.licenses) == AGPL-3.0-only ]] || missing+=" licenses"
[[ -z $missing ]] && ok labels || fail labels "wrong or missing:$missing"

got=$(docker run --rm "$image" --version 2>&1)
[[ $got == "cairn $version" ]] && ok version || fail version "printed '$got', want 'cairn $version'"

# Health: healthy when serving, exit 1 when nothing listens
if docker run -d --name "$name-run" --health-interval=2s --health-start-period=0s "$image" >/dev/null 2>&1; then
  status=""
  for _ in $(seq 30); do
    status=$(docker inspect -f '{{.State.Health.Status}}' "$name-run" 2>/dev/null)
    [[ $status == healthy ]] && break
    sleep 1
  done
  [[ $status == healthy ]] && ok healthy || fail healthy "status '$status' after 30 s"
  if docker cp "$name-run:/data" - 2>/dev/null | tar -tv 2>/dev/null | grep -q -E '^-.* 10001/10001 .* data/cairn\.db$'; then
    ok db_owner
  else
    fail db_owner "/data/cairn.db missing or not owned by 10001"
  fi
else
  fail healthy "cannot start $image"
  fail db_owner "cannot start $image"
fi

docker run --rm --entrypoint /cairn "$image" health >/dev/null 2>&1
rc=$?
[[ $rc == 1 ]] && ok health_fails_without_server || fail health_fails_without_server "exit $rc, want 1"

exit "$failed"
