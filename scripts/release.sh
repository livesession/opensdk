#!/usr/bin/env bash
# Cut a release: bump, verify, commit, tag.
#
#   scripts/release.sh 0.1.2
#   scripts/release.sh 0.2.0-rc1
#
# release.yml refuses to publish a tag that disagrees with cli/Cargo.toml, and
# the ordering that satisfies it is easy to get backwards: the tag must point at
# a commit whose manifest ALREADY says the new version. Tagging first, or
# bumping without committing, fails in CI minutes later rather than here.
#
# Nothing is pushed. The script prints the two push commands and stops.
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:-}"
if [ -z "$version" ]; then
  echo "usage: scripts/release.sh <version>   e.g. 0.1.2 or 0.2.0-rc1" >&2
  exit 1
fi
# Strip a leading v so both `0.1.2` and `v0.1.2` work; the tag gets it back.
version="${version#v}"
tag="v${version}"

# Semver-ish, prerelease allowed. release.yml compares EXACTLY, so `0.2.0-rc1`
# in the tag needs `0.2.0-rc1` here too — not the `0.2.0` core.
if ! printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'; then
  echo "error: '$version' is not a version cargo will accept" >&2
  exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
  echo "error: working tree is dirty — commit or stash first" >&2
  git status --short >&2
  exit 1
fi

if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "error: tag $tag already exists locally." >&2
  echo "       To move it (e.g. after a failed run):  git tag -d $tag" >&2
  exit 1
fi

current=$(grep -m1 '^version = ' cli/Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
echo "cli/Cargo.toml: $current -> $version"

# Only the [package] version, which is the first `version = ` in the file — the
# dependency pins further down must not be touched.
perl -0pi -e "s/^version = \"\Q$current\E\"\$/version = \"$version\"/m" cli/Cargo.toml

# Refresh Cargo.lock's entry for this package. `cargo check`, NOT
# `generate-lockfile`: the latter re-resolves every transitive dependency, and
# nearly every gate here is a byte comparison that an unrelated bump can flip.
cargo check -q -p opensdk

changed=$(git diff --numstat Cargo.lock | awk '{print $1+$2}')
if [ "${changed:-0}" -gt 2 ]; then
  echo "error: Cargo.lock moved by $changed lines; expected 2 (the version)." >&2
  echo "       A transitive dependency re-resolved — inspect before releasing:" >&2
  git --no-pager diff --stat Cargo.lock >&2
  exit 1
fi

# The same comparison release.yml runs, run here where it costs seconds.
manifest=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c "import json,sys;print(next(p['version'] for p in json.load(sys.stdin)['packages'] if p['name']=='opensdk'))")
if [ "$manifest" != "$version" ]; then
  echo "error: manifest reads $manifest after the bump, expected $version" >&2
  exit 1
fi

git add cli/Cargo.toml Cargo.lock
git commit -q -m "chore(release): $tag"
git tag -a "$tag" -m "$tag"

echo
echo "committed and tagged $tag. Nothing pushed yet:"
echo
echo "    git push origin master"
echo "    git push origin $tag"
echo
case "$version" in
  *-*) echo "note: '$version' is a PRERELEASE. GitHub's /releases/latest/ skips"
       echo "      prereleases, so xyd's installer will not see it." ;;
esac
