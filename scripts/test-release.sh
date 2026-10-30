#!/usr/bin/env bash
# Tests the release scripts: the tag check, a rehearsal of the image push and merge against a throwaway registry,
# and actionlint on the workflows. One "ok" or "FAIL" line per case.
# Usage: scripts/test-release.sh [image-for-nothing]  (needs Docker; uses 127.0.0.1:15000)
set -uo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
work=$(mktemp -d)
registry=cairn-rehearsal
cleanup() {
  docker rm -f "$registry" >/dev/null 2>&1 || true
  docker buildx rm "$registry" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

failed=0
ok() { echo "ok $1"; }
fail() { echo "FAIL $1: $2"; failed=1; }

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
minor=${version%.*}

# Tag check
out=$(scripts/release-version.sh "v$version" 2>&1)
want=$(printf 'version=%s\nminor=%s\nprerelease=false' "$version" "$minor")
[[ $out == "$want" ]] && ok tag_matches || fail tag_matches "$out"

out=$(scripts/release-version.sh v99.0.1 2>&1)
rc=$?
[[ $rc == 1 && $out == "The tag v99.0.1 does not match version $version in Cargo.toml." ]] \
  && ok tag_differs || fail tag_differs "exit $rc: $out"

scripts/release-version.sh "$version" >/dev/null 2>&1
rc=$?
[[ $rc == 1 ]] && ok tag_without_v || fail tag_without_v "exit $rc"

sed 's/^version = ".*"$/version = "0.2.0-rc.1"/' Cargo.toml > "$work/Cargo.toml"
out=$(CARGO_TOML="$work/Cargo.toml" scripts/release-version.sh v0.2.0-rc.1 2>&1)
want=$(printf 'version=0.2.0-rc.1\nminor=0.2\nprerelease=true')
[[ $out == "$want" ]] && ok prerelease || fail prerelease "$out"

# Rehearsal: push by digest and merge, as the workflow does, into a local registry
rehearsal() {
  local repo=127.0.0.1:15000/cairn digest a b c
  docker run -d --name "$registry" -p 127.0.0.1:15000:5000 \
    registry@sha256:a3d8aaa63ed8681a604f1dea0aa03f100d5895b6a58ace528858a7b332415373 >/dev/null || return 1
  # Push by digest needs a docker-container builder, as in the workflow; it reaches the registry over the host network.
  printf '[registry."127.0.0.1:15000"]\n  http = true\n' > "$work/buildkitd.toml"
  docker buildx create --name "$registry" --driver docker-container --driver-opt network=host \
    --buildkitd-config "$work/buildkitd.toml" >/dev/null || return 1
  export BUILDX_BUILDER=$registry
  digest=$(SOURCE_URL=https://example.com/cairn VERSION="$version" REVISION=$(git rev-parse HEAD) \
    scripts/release-image.sh push "$repo" linux/amd64 2> "$work/push.log" | tail -1)
  [[ $digest == sha256:* ]] || { echo "push printed '$digest'"; tail -20 "$work/push.log"; return 1; }
  scripts/release-image.sh merge "$repo" "$version" "$minor" false "$digest" > "$work/merge.log" 2>&1 \
    || { tail -20 "$work/merge.log"; return 1; }
  for tag in "$version" "$minor" latest; do
    docker pull -q "$repo:$tag" >/dev/null || { echo "cannot pull $repo:$tag"; return 1; }
  done
  a=$(docker image inspect -f '{{index .RepoDigests 0}}' "$repo:$version")
  b=$(docker buildx imagetools inspect "$repo:$minor" --format '{{.Manifest.Digest}}')
  c=$(docker buildx imagetools inspect "$repo:latest" --format '{{.Manifest.Digest}}')
  [[ ${a#*@} == "$b" && $b == "$c" ]] || { echo "digests differ: $a $b $c"; return 1; }
  [[ $(docker image inspect -f '{{index .Config.Labels "org.opencontainers.image.version"}}' "$repo:$version") == "$version" ]] \
    || { echo "version label missing"; return 1; }
  scripts/check-image.sh "$repo:$version" > "$work/check.log" || { cat "$work/check.log"; return 1; }
  scripts/release-image.sh merge "$repo" 0.2.0-rc.1 0.2 true "$digest" > "$work/merge2.log" 2>&1 \
    || { tail -20 "$work/merge2.log"; return 1; }
  [[ $(docker buildx imagetools inspect "$repo:latest" --format '{{.Manifest.Digest}}') == "$c" ]] \
    || { echo "a pre-release moved latest"; return 1; }
  if docker buildx imagetools inspect "$repo:0.2" >/dev/null 2>&1; then echo "a pre-release made the minor tag"; return 1; fi
  docker image rm "$repo:$version" "$repo:$minor" "$repo:latest" >/dev/null 2>&1 || true
}
if msg=$(rehearsal 2>&1); then ok rehearsal; else fail rehearsal "$msg"; fi

# actionlint
if msg=$(docker run --rm -v "$root:/repo" -w /repo \
  rhysd/actionlint@sha256:b1934ee5f1c509618f2508e6eb47ee0d3520686341fec936f3b79331f9315667 \
  .github/workflows/ci.yml .github/workflows/release.yml 2>&1); then
  ok actionlint
else
  fail actionlint "$msg"
fi

exit "$failed"
