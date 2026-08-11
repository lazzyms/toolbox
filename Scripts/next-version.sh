#!/bin/bash
#
# Prints the version the next release should carry.
#
#   ./Scripts/next-version.sh                      # patch bump: 1.0.0 -> 1.0.1
#   BUMP=minor ./Scripts/next-version.sh           #             1.0.0 -> 1.1.0
#   LABELS="major images" ./Scripts/next-version.sh # a PR label picks the bump
#   VERSION=2.0.0 ./Scripts/next-version.sh        # validated passthrough
#   ./Scripts/next-version.sh --previous           # the version already released
#
# Git tags are the source of truth, not a file in the tree: the release commit and
# its tag are pushed together, so there is no second place to keep in sync — and
# nothing for a contributor's PR to bump by mistake.
#
set -euo pipefail

cd "$(dirname "$0")/.."

MODE="next"
if [[ "${1:-}" == "--previous" ]]; then
  MODE="previous"
elif [[ $# -gt 0 ]]; then
  echo "unknown option: $1" >&2
  exit 1
fi

# -v:refname sorts version components numerically, so v1.10.0 beats v1.9.0 —
# plain lexical sort gets that backwards.
LATEST="$(git tag --list 'v[0-9]*' --sort=-v:refname | head -1)"
CURRENT="${LATEST#v}"

if [[ "$MODE" == "previous" ]]; then
  # Empty when nothing has been released yet; callers treat that as "no range".
  echo "$CURRENT"
  exit 0
fi

# An explicit version wins over any bump, but is still checked: a typo here would
# be baked into a signed feed that clients compare against.
if [[ -n "${VERSION:-}" ]]; then
  if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: VERSION must look like 1.2.3, got '$VERSION'" >&2
    exit 1
  fi
  echo "$VERSION"
  exit 0
fi

BUMP="${BUMP:-patch}"

# A label on the merged PR overrides the default. `major` wins when both are
# present: releasing a smaller bump than was asked for is the worse mistake, and
# it can't be taken back once clients have seen the tag.
case " ${LABELS:-} " in
  *" major "*|*" release:major "*) BUMP="major" ;;
  *" minor "*|*" release:minor "*) BUMP="minor" ;;
esac

# Nothing tagged yet, so there is nothing to bump from.
if [[ -z "$CURRENT" ]]; then
  echo "1.0.0"
  exit 0
fi

if ! [[ "$CURRENT" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
  echo "error: latest tag '$LATEST' is not a three-part version" >&2
  exit 1
fi
MAJOR="${BASH_REMATCH[1]}"
MINOR="${BASH_REMATCH[2]}"
PATCH="${BASH_REMATCH[3]}"

case "$BUMP" in
  major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
  minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
  patch) PATCH=$((PATCH + 1)) ;;
  *) echo "error: BUMP must be major, minor or patch, got '$BUMP'" >&2; exit 1 ;;
esac

echo "$MAJOR.$MINOR.$PATCH"
