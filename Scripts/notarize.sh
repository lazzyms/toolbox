#!/bin/bash
#
# Notarizes the DMG so it opens with a normal double-click on any Mac, with no
# right-click workaround and no Gatekeeper warning.
#
# REQUIREMENTS (one-time)
#   1. Apple Developer Program membership ($99/year)
#   2. A "Developer ID Application" certificate in your login keychain
#      (Xcode → Settings → Accounts → Manage Certificates → + → Developer ID Application)
#   3. An app-specific password from https://appleid.apple.com → Sign-In and Security
#   4. Store the credentials once:
#        xcrun notarytool store-credentials "toolbox-notary" \
#          --apple-id "you@example.com" \
#          --team-id "YOURTEAMID" \
#          --password "abcd-efgh-ijkl-mnop"
#
# USAGE
#   ./Scripts/build-app.sh --version 1.0.0 --sign auto
#   ./Scripts/make-dmg.sh  --version 1.0.0
#   ./Scripts/notarize.sh  --version 1.0.0
#
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

VERSION="1.0.0"
PROFILE="toolbox-notary"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --profile) PROFILE="$2"; shift 2 ;;
    *) echo "unknown option: $1" >&2; exit 1 ;;
  esac
done

DMG="$ROOT/dist/Toolbox-$VERSION.dmg"
APP="$ROOT/dist/Toolbox.app"

if [[ ! -f "$DMG" ]]; then
  echo "error: $DMG not found — run ./Scripts/make-dmg.sh first" >&2
  exit 1
fi

# Notarization rejects ad-hoc signatures outright, so fail early with a clear
# message rather than after a slow upload.
echo "==> Checking the signature"
if ! codesign -dvv "$APP" 2>&1 | grep -q "Authority=Developer ID Application"; then
  echo "" >&2
  echo "error: the app is not signed with a Developer ID Application certificate." >&2
  echo "       Apple will reject the notarization request." >&2
  echo "" >&2
  echo "       Rebuild with:  ./Scripts/build-app.sh --version $VERSION --sign auto" >&2
  echo "       If that still says 'no certificate found', you need an active" >&2
  echo "       Apple Developer Program membership." >&2
  exit 1
fi

# The Hardened Runtime is mandatory for notarization.
if ! codesign -d --entitlements - "$APP" 2>/dev/null | grep -q . ; then
  echo "    note: no entitlements found (fine, but unusual)"
fi
codesign -dvv "$APP" 2>&1 | grep -q "flags=.*runtime" \
  || { echo "error: Hardened Runtime missing — rebuild with build-app.sh" >&2; exit 1; }

echo "==> Submitting to Apple (this usually takes 1–5 minutes)"
xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" --wait

echo "==> Stapling the ticket"
# Stapling embeds the approval so the DMG validates even offline.
xcrun stapler staple "$DMG"

echo "==> Verifying"
xcrun stapler validate "$DMG"
spctl --assess --type open --context context:primary-signature -vv "$DMG" 2>&1 || true

echo ""
echo "Notarized $DMG"
echo "SHA-256: $(shasum -a 256 "$DMG" | cut -d' ' -f1)"
echo "This DMG now opens on any Mac with a normal double-click."
