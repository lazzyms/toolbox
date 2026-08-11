#!/bin/bash
#
# Cuts a release that installed copies of Toolbox will auto-update to.
#
#   ./Scripts/release.sh --version 1.1.0 [--notes "path/to/notes.md"]
#
# What it does:
#   1. builds the universal app and DMG
#   2. signs the DMG with the Ed25519 private key
#   3. writes/updates docs/appcast.xml — the feed the app polls
#   4. creates a GitHub release with the DMG attached
#
# The key comes from your login Keychain, or from $SPARKLE_PRIVATE_KEY when set —
# that is how .github/workflows/release.yml signs, since a runner has no Keychain
# to unlock. Normally you don't run this by hand: merging to main runs it in CI.
#
# The appcast is served from GitHub Pages (docs/ on the default branch), which
# must match SUFeedURL in Resources/Info.plist.
#
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

# One source of truth: the download URL baked into the signed feed must match the
# repo the release is actually created in, or clients download a 404.
REPO="lazzyms/toolbox"

VERSION=""
NOTES=""
DRY_RUN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --notes)   NOTES="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    *) echo "unknown option: $1" >&2; exit 1 ;;
  esac
done

if [[ -z "$VERSION" ]]; then
  echo "error: --version is required (e.g. --version 1.1.0)" >&2
  exit 1
fi
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: version must look like 1.2.3" >&2
  exit 1
fi

SPARKLE_BIN="$(find "$ROOT/.build/artifacts" -type d -name bin -path "*sparkle*" | head -1)"
if [[ ! -x "$SPARKLE_BIN/generate_appcast" ]]; then
  echo "error: Sparkle tools missing — run 'swift package resolve'" >&2
  exit 1
fi

# Sparkle compares CFBundleVersion (the build number) to decide what is newer, so
# a monotonic value is required. Commit count works and needs no manual tracking.
BUILD_NUMBER="$(git rev-list --count HEAD)"

echo "=========================================="
echo " Toolbox $VERSION (build $BUILD_NUMBER)"
echo "=========================================="

# --- 1. Build ------------------------------------------------------------
echo ""
echo "### Building"
"$ROOT/Scripts/build-app.sh" --version "$VERSION" --sign auto

echo ""
echo "### Packaging DMG"
"$ROOT/Scripts/make-dmg.sh" --version "$VERSION"

DMG="$ROOT/dist/Toolbox-$VERSION.dmg"
[[ -f "$DMG" ]] || { echo "error: $DMG not produced" >&2; exit 1; }

# --- 2. Appcast ----------------------------------------------------------
# generate_appcast wants a directory of archives; give it one holding just the
# DMGs so it doesn't try to interpret build leftovers.
echo ""
echo "### Signing and generating appcast"
FEED_DIR="$ROOT/dist/feed"
mkdir -p "$FEED_DIR"
cp "$DMG" "$FEED_DIR/"

# Carry the existing feed forward so older versions stay listed and users on any
# version still see a valid upgrade path.
if [[ -f "$ROOT/docs/appcast.xml" ]]; then
  cp "$ROOT/docs/appcast.xml" "$FEED_DIR/appcast.xml"
fi

# Release notes: same basename as the DMG, so Sparkle shows them in its dialog.
if [[ -n "$NOTES" && -f "$NOTES" ]]; then
  cp "$NOTES" "$FEED_DIR/Toolbox-$VERSION.md"
fi

DOWNLOAD_PREFIX="https://github.com/$REPO/releases/download/v$VERSION"

# Signs each archive with the private key and embeds the signature plus length in
# the feed. Fails loudly if the key is missing rather than emitting an unsigned
# feed that clients would reject.
APPCAST_ARGS=(
  --download-url-prefix "$DOWNLOAD_PREFIX/"
  --link "https://github.com/$REPO"
  --embed-release-notes
)

if [[ -n "${SPARKLE_PRIVATE_KEY:-}" ]]; then
  # CI has no Keychain to unlock, so the key arrives as a secret. `--ed-key-file -`
  # reads it from stdin, which keeps it off disk and out of the process list.
  echo "    signing with \$SPARKLE_PRIVATE_KEY"
  printf '%s' "$SPARKLE_PRIVATE_KEY" \
    | "$SPARKLE_BIN/generate_appcast" "${APPCAST_ARGS[@]}" --ed-key-file - "$FEED_DIR"
else
  echo "    signing with the private key in your login Keychain"
  "$SPARKLE_BIN/generate_appcast" "${APPCAST_ARGS[@]}" "$FEED_DIR"
fi

mkdir -p "$ROOT/docs"
cp "$FEED_DIR/appcast.xml" "$ROOT/docs/appcast.xml"

# Verify the feed actually carries a signature — an unsigned entry would be
# rejected by every client and is the one mistake worth catching here.
if ! grep -q 'sparkle:edSignature' "$ROOT/docs/appcast.xml"; then
  echo "error: appcast has no EdDSA signature — clients would reject this update." >&2
  if [[ -n "${SPARKLE_PRIVATE_KEY:-}" ]]; then
    echo "       \$SPARKLE_PRIVATE_KEY is set but was not accepted. It must be the" >&2
    echo "       exact contents of 'generate_keys -x', with no extra whitespace." >&2
  else
    echo "       Is the private key in your login Keychain? Run:" >&2
    echo "         $SPARKLE_BIN/generate_keys" >&2
  fi
  exit 1
fi
echo "    appcast signed and written to docs/appcast.xml"

if [[ "$DRY_RUN" == "1" ]]; then
  echo ""
  echo "Dry run — stopping before the GitHub release."
  echo "Feed preview:"
  sed 's/^/    /' "$ROOT/docs/appcast.xml"
  exit 0
fi

# --- 3. Publish ----------------------------------------------------------
echo ""
echo "### Publishing to GitHub"

# GitHub Pages serves docs/ from the *default* branch, so the appcast has to land
# there or SUFeedURL 404s no matter what was committed. Pushing the current
# branch is not enough when releasing from a topic branch.
DEFAULT_BRANCH="$(gh repo view "$REPO" --json defaultBranchRef \
  -q .defaultBranchRef.name 2>/dev/null || echo main)"

git add docs/appcast.xml
git commit -q -m "Release $VERSION" || echo "    (nothing to commit)"
git tag -f "v$VERSION"
# Fails rather than force-pushing if the default branch has diverged: silently
# overwriting someone else's commits is far worse than a stopped release.
if ! git push origin "HEAD:$DEFAULT_BRANCH"; then
  echo "" >&2
  echo "error: could not fast-forward $DEFAULT_BRANCH. Rebase onto it and retry;" >&2
  echo "       the appcast must be on $DEFAULT_BRANCH for GitHub Pages to serve it." >&2
  exit 1
fi
git push origin -f "v$VERSION"

RELEASE_NOTES_ARG=()
if [[ -n "$NOTES" && -f "$NOTES" ]]; then
  RELEASE_NOTES_ARG=(--notes-file "$NOTES")
else
  RELEASE_NOTES_ARG=(--generate-notes)
fi

gh release create "v$VERSION" "$DMG" \
  --title "Toolbox $VERSION" \
  "${RELEASE_NOTES_ARG[@]}" \
  --repo "$REPO"

# The feed is useless if the asset URL it points at doesn't resolve, and that is
# exactly the mistake a signed appcast hides until a user tries to update.
echo ""
echo "### Verifying the published download"
ASSET_URL="$DOWNLOAD_PREFIX/Toolbox-$VERSION.dmg"
CODE="$(curl -sIL -o /dev/null -w '%{http_code}' "$ASSET_URL")"
if [[ "$CODE" != "200" ]]; then
  echo "error: $ASSET_URL returned HTTP $CODE — the appcast points at a broken URL." >&2
  exit 1
fi
echo "    $ASSET_URL → HTTP 200"

echo ""
echo "=========================================="
echo " Released Toolbox $VERSION"
echo "=========================================="
echo "DMG:     $DMG"
echo "SHA-256: $(shasum -a 256 "$DMG" | cut -d' ' -f1)"
echo ""
echo "Installed copies will offer this update within 24 hours,"
echo "or immediately via Toolbox → Check for Updates…"
echo ""
echo "If this is your first release, enable GitHub Pages once:"
echo "  Settings → Pages → Source: deploy from branch → main → /docs"
