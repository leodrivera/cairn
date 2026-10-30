#!/usr/bin/env bash
# Checks that a release tag is "v" plus the version in Cargo.toml, and prints the values the release needs, as
# lines for $GITHUB_OUTPUT: version, minor (major.minor) and prerelease (true when the version has a "-").
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
echo "version=$version"
echo "minor=${core%.*}"
if [[ $version == *-* ]]; then echo "prerelease=true"; else echo "prerelease=false"; fi
