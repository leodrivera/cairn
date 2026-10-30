#!/usr/bin/env bash
# The two halves of publishing the image, shared by the release workflow and its local rehearsal.
#
#   scripts/release-image.sh push <repo> <platform>
#     Builds the Dockerfile for <platform> on a machine of that architecture, runs check-image.sh on it, pushes it
#     by digest only (no tag) and prints the digest last. Labels come from SOURCE_URL, VERSION and REVISION.
#   scripts/release-image.sh merge <repo> <version> <minor> <prerelease> <digest>...
#     Joins the per-platform digests into one manifest tagged <version>, plus <minor> and latest unless
#     <prerelease> is true.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
mode=${1:?usage: release-image.sh push|merge ...}
repo=${2:?missing repository}

case "$mode" in
  push)
    platform=${3:?missing platform}
    : "${SOURCE_URL:?}" "${VERSION:?}" "${REVISION:?}"
    labels=(
      --label "org.opencontainers.image.source=$SOURCE_URL"
      --label "org.opencontainers.image.version=$VERSION"
      --label "org.opencontainers.image.revision=$REVISION"
    )
    local_tag="cairn-release:$VERSION-${platform//\//-}"
    docker buildx build --platform "$platform" "${labels[@]}" --load -t "$local_tag" "$root" >&2
    "$root/scripts/check-image.sh" "$local_tag" >&2
    metadata=$(mktemp)
    docker buildx build --platform "$platform" "${labels[@]}" \
      --output "type=image,name=$repo,push-by-digest=true,name-canonical=true,push=true" \
      --metadata-file "$metadata" "$root" >&2
    jq -r ".\"containerimage.digest\"" "$metadata"
    rm -f "$metadata"
    ;;
  merge)
    version=${3:?missing version}
    minor=${4:?missing minor}
    prerelease=${5:?missing prerelease}
    shift 5
    (( $# > 0 )) || { echo "no digests" >&2; exit 1; }
    tags=(-t "$repo:$version")
    if [[ $prerelease != true ]]; then tags+=(-t "$repo:$minor" -t "$repo:latest"); fi
    sources=()
    for digest in "$@"; do sources+=("$repo@$digest"); done
    docker buildx imagetools create "${tags[@]}" "${sources[@]}"
    ;;
  *)
    echo "usage: release-image.sh push|merge ..." >&2
    exit 2
    ;;
esac
