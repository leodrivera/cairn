#!/usr/bin/env bash
# Checks that a release tag is "v" plus the version in Cargo.toml, and prints the values the release needs, as
# lines for $GITHUB_OUTPUT:
#   version       the version
#   minor         major.minor
#   prerelease    true when the version has a "-"
#   latest        true when this is the highest release so far, so `latest` may move to it
#   minor_latest  true when this is the highest release of its major.minor, so that tag may move to it
# Pre-releases never move `latest` or the minor tag, and are left out of the comparison. The release tags come from
# `git tag -l 'v*'`, or from TAGS (space-separated) when set.
# Usage: scripts/release-version.sh <tag>
set -euo pipefail

tag=${1:?usage: scripts/release-version.sh <tag>}
cargo_toml=${CARGO_TOML:-$(cd "$(dirname "$0")/.." && pwd)/Cargo.toml}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$cargo_toml" | head -1)

if [[ $tag != "v$version" ]]; then
  echo "The tag $tag does not match version $version in Cargo.toml." >&2
  exit 1
fi
core=${version%%-*}
minor=${core%.*}
echo "version=$version"
echo "minor=$minor"
if [[ $version == *-* ]]; then
  echo "prerelease=true"
  echo "latest=false"
  echo "minor_latest=false"
  exit 0
fi
echo "prerelease=false"

tags=${TAGS-$(git -C "$(dirname "$cargo_toml")" tag -l 'v*')}
# Releases only, without the "v", plus this one, highest first.
releases=$( { tr ' ' '\n' <<< "$tags"; echo "v$version"; } | sed -n 's/^v\([0-9]*\.[0-9]*\.[0-9]*\)$/\1/p' | sort -u -V -r)
highest=$(head -1 <<< "$releases")
highest_minor=$(grep "^${minor//./\\.}\\." <<< "$releases" | head -1)
[[ $highest == "$version" ]] && echo "latest=true" || echo "latest=false"
[[ $highest_minor == "$version" ]] && echo "minor_latest=true" || echo "minor_latest=false"
