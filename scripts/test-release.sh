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
out=$(TAGS="v$version" scripts/release-version.sh "v$version" 2>&1)
want=$(printf 'version=%s\nminor=%s\nprerelease=false\nlatest=true\nminor_latest=true' "$version" "$minor")
[[ $out == "$want" ]] && ok tag_matches || fail tag_matches "$out"

# latest and the minor tag move only for the highest release; pre-releases never count.
moves() { # cargo-version tag tags... -> "latest minor_latest"
  local v=$1 tag=$2
  shift 2
  sed "s/^version = \".*\"$/version = \"$v\"/" Cargo.toml > "$work/Cargo.toml"
  CARGO_TOML="$work/Cargo.toml" TAGS="$*" scripts/release-version.sh "$tag" \
    | sed -n 's/^\(latest\|minor_latest\)=//p' | tr '\n' ' '
}
[[ $(moves 0.1.1 v0.1.1 v0.1.0 v0.2.0 v0.1.1) == "false true " ]] && ok older_patch_keeps_latest \
  || fail older_patch_keeps_latest "$(moves 0.1.1 v0.1.1 v0.1.0 v0.2.0 v0.1.1)"
[[ $(moves 0.1.1 v0.1.1 v0.1.0 v0.1.2 v0.2.0 v0.1.1) == "false false " ]] && ok older_patch_keeps_minor \
  || fail older_patch_keeps_minor "$(moves 0.1.1 v0.1.1 v0.1.0 v0.1.2 v0.2.0 v0.1.1)"
[[ $(moves 0.2.1 v0.2.1 v0.1.0 v0.2.0 v0.3.0-rc.1 v0.2.1) == "true true " ]] && ok prerelease_tags_ignored \
  || fail prerelease_tags_ignored "$(moves 0.2.1 v0.2.1 v0.1.0 v0.2.0 v0.3.0-rc.1 v0.2.1)"
[[ $(moves 0.10.0 v0.10.0 v0.9.0 v0.10.0) == "true true " ]] && ok versions_compare_as_numbers \
  || fail versions_compare_as_numbers "$(moves 0.10.0 v0.10.0 v0.9.0 v0.10.0)"

out=$(scripts/release-version.sh v99.0.1 2>&1)
rc=$?
[[ $rc == 1 && $out == "The tag v99.0.1 does not match version $version in Cargo.toml." ]] \
  && ok tag_differs || fail tag_differs "exit $rc: $out"

scripts/release-version.sh "$version" >/dev/null 2>&1
rc=$?
[[ $rc == 1 ]] && ok tag_without_v || fail tag_without_v "exit $rc"

sed 's/^version = ".*"$/version = "0.2.0-rc.1"/' Cargo.toml > "$work/Cargo.toml"
out=$(CARGO_TOML="$work/Cargo.toml" TAGS="v0.1.0 v0.2.0-rc.1" scripts/release-version.sh v0.2.0-rc.1 2>&1)
want=$(printf 'version=0.2.0-rc.1\nminor=0.2\nprerelease=true\nlatest=false\nminor_latest=false')
[[ $out == "$want" ]] && ok prerelease || fail prerelease "$out"

# Rehearsal: push by digest and merge, as the workflow does, into a local registry. The push checks the budget,
# which measures the front-end in web/dist.
[[ -d web/dist/assets ]] || { echo "FAIL rehearsal: build web/dist first (cd web && npm run build)"; exit 1; }
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
  # What was checked is what was pushed: the image pulled back by its digest.
  grep -qxF "checked $repo@$digest" "$work/push.log" || { echo "the pushed digest was not the one checked"; return 1; }
  scripts/release-image.sh merge "$repo" "$version" "$minor" true true "$digest" > "$work/merge.log" 2>&1 \
    || { tail -20 "$work/merge.log"; return 1; }
  # One entry per platform and nothing else (no attestation entries that ghcr lists as unknown/unknown).
  local platforms
  platforms=$(docker buildx imagetools inspect --raw "$repo:$version" \
    | jq -r '.manifests[] | "\(.platform.os)/\(.platform.architecture)"' | sort | tr '\n' ' ')
  [[ $platforms == "linux/amd64 " ]] || { echo "manifest lists: $platforms"; return 1; }
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
  # A second image with another revision, so that a wrongly moved tag would change digest.
  local other
  other=$(SOURCE_URL=https://example.com/cairn VERSION=0.2.0-rc.1 REVISION=0000000 \
    scripts/release-image.sh push "$repo" linux/amd64 2> "$work/push2.log" | tail -1)
  [[ $other == sha256:* && $other != "$digest" ]] || { echo "second push printed '$other'"; return 1; }
  scripts/release-image.sh merge "$repo" 0.2.0-rc.1 0.2 false false "$other" > "$work/merge2.log" 2>&1 \
    || { tail -20 "$work/merge2.log"; return 1; }
  [[ $(docker buildx imagetools inspect "$repo:latest" --format '{{.Manifest.Digest}}') == "$c" ]] \
    || { echo "a pre-release moved latest"; return 1; }
  if docker buildx imagetools inspect "$repo:0.2" >/dev/null 2>&1; then echo "a pre-release made the minor tag"; return 1; fi
  docker image rm "$repo:$version" "$repo:$minor" "$repo:latest" >/dev/null 2>&1 || true
}
if msg=$(rehearsal 2>&1); then ok rehearsal; else fail rehearsal "$msg"; fi

# The release runs ci.yml's checks before it builds, one release at a time.
if msg=$(python3 -I - .github/workflows/release.yml .github/workflows/ci.yml <<'PY' 2>&1
import sys, yaml
release, ci = (yaml.safe_load(open(p)) for p in sys.argv[1:3])
ci_on = ci.get(True, ci.get("on"))
problems = []
if "workflow_call" not in ci_on: problems.append("ci.yml cannot be called")
if release["jobs"].get("test", {}).get("uses") != "./.github/workflows/ci.yml": problems.append("no test job calling ci.yml")
needs = release["jobs"]["build"]["needs"]
if "test" not in (needs if isinstance(needs, list) else [needs]): problems.append("build does not need test")
if release.get("concurrency", {}).get("cancel-in-progress") is not False: problems.append("no release concurrency group")
# The release checks its own pushed images, so the called CI skips its image job.
if (release["jobs"]["test"].get("with") or {}).get("image") is not False: problems.append("release test does not pass image: false")
if (((ci_on.get("workflow_call") or {}).get("inputs") or {}).get("image") or {}).get("type") != "boolean": problems.append("ci.yml has no boolean image input")
# `inputs.image != false` would skip the job on push and pull_request too: null and false both coerce to 0.
if "toJSON(inputs.image) != 'false'" not in str(ci["jobs"]["image"].get("if", "")): problems.append("ci image job does not honor the image input")
# The budget measures web/dist, so the build job builds the front-end.
if not any("npm run build" in step.get("run", "") for step in release["jobs"]["build"]["steps"]): problems.append("release build does not build web/dist")
print("; ".join(problems))
sys.exit(1 if problems else 0)
PY
); then ok release_runs_ci_first; else fail release_runs_ci_first "$msg"; fi

# The pushed image is the one whose budget is checked.
grep -q 'check-budget.sh" "$repo@$digest"' scripts/release-image.sh && ok release_checks_budget \
  || fail release_checks_budget "release-image.sh push does not run check-budget.sh on the pushed digest"

# CI runs once per change: pushes to main, pull requests, by hand, or called by the release; never for a tag push.
if msg=$(python3 -I - .github/workflows/ci.yml <<'PY' 2>&1
import sys, yaml
ci = yaml.safe_load(open(sys.argv[1]))
on = ci.get(True, ci.get("on"))
problems = []
push = on.get("push") or {}
if push.get("branches") != ["main"] or "tags" in push: problems.append(f"push filter is {push!r}, want branches [main]")
for key in ("pull_request", "workflow_dispatch", "workflow_call"):
    if key not in on: problems.append(f"no {key}")
for key in ("paths", "paths-ignore"):
    if any(key in (on.get(e) or {}) for e in ("push", "pull_request")): problems.append(f"{key} would skip check-readme")
conc = ci.get("concurrency") or {}
if "github.ref" not in str(conc.get("group", "")) or "refs/heads/main" not in str(conc.get("cancel-in-progress", "")):
    problems.append("concurrency must cancel superseded runs everywhere but main")
for name, job in ci["jobs"].items():
    if not job.get("timeout-minutes"): problems.append(f"{name} has no timeout")
if not any("check-readme.sh" in s.get("run", "") for s in ci["jobs"]["test"]["steps"]): problems.append("test does not run check-readme.sh")
print("; ".join(problems))
sys.exit(1 if problems else 0)
PY
); then ok ci_runs_once; else fail ci_runs_once "$msg"; fi

# Every release job that runs steps has a timeout.
if msg=$(python3 -I - .github/workflows/release.yml <<'PY' 2>&1
import sys, yaml
jobs = yaml.safe_load(open(sys.argv[1]))["jobs"]
bad = [n for n, j in jobs.items() if "uses" not in j and not j.get("timeout-minutes")]
print("no timeout: " + ", ".join(bad) if bad else "")
sys.exit(1 if bad else 0)
PY
); then ok release_timeouts; else fail release_timeouts "$msg"; fi

# Secrets reach scripts through env:, never pasted into the script text.
if msg=$(python3 -I - .github/workflows/release.yml .github/workflows/ci.yml <<'PY' 2>&1
import sys, yaml
bad = []
for path in sys.argv[1:]:
    for name, job in (yaml.safe_load(open(path)).get("jobs") or {}).items():
        for step in job.get("steps", []):
            if "${{ secrets." in step.get("run", ""):
                bad.append(f"{path}: {name}: {step.get('name', '?')}")
print("; ".join(bad))
sys.exit(1 if bad else 0)
PY
); then ok secrets_through_env; else fail secrets_through_env "$msg"; fi

# actionlint
if msg=$(docker run --rm -v "$root:/repo" -w /repo \
  rhysd/actionlint@sha256:b1934ee5f1c509618f2508e6eb47ee0d3520686341fec936f3b79331f9315667 \
  .github/workflows/ci.yml .github/workflows/release.yml 2>&1); then
  ok actionlint
else
  fail actionlint "$msg"
fi

exit "$failed"
