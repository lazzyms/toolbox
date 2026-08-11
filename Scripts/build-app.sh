#!/bin/bash
#
# Builds Toolbox.app as a universal (Apple Silicon + Intel) release bundle.
#
#   ./Scripts/build-app.sh [--version 1.0.0] [--sign "identity"]
#
# Signing:
#   --sign auto    use a Developer ID Application cert if one exists (best)
#   --sign adhoc   ad-hoc signature, no certificate needed (default)
#   --sign "Developer ID Application: Name (TEAMID)"   explicit identity
#
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

VERSION="1.0.0"
BUILD_NUMBER="$(git rev-list --count HEAD 2>/dev/null || echo 1)"
SIGN_MODE="adhoc"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --sign)    SIGN_MODE="$2"; shift 2 ;;
    *) echo "unknown option: $1" >&2; exit 1 ;;
  esac
done

APP="$ROOT/dist/Toolbox.app"
CONTENTS="$APP/Contents"

echo "==> Cleaning"
rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"

echo "==> Generating icon"
swift "$ROOT/Scripts/make-icon.swift" "$ROOT/Resources" >/dev/null

# Build each slice separately, then lipo them. Building both arches in one
# swift build invocation is not supported for executables.
for ARCH in arm64 x86_64; do
  echo "==> Building $ARCH"
  swift build -c release --arch "$ARCH" >/dev/null
done

echo "==> Creating universal binary"
lipo -create \
  "$ROOT/.build/arm64-apple-macosx/release/Toolbox" \
  "$ROOT/.build/x86_64-apple-macosx/release/Toolbox" \
  -output "$CONTENTS/MacOS/Toolbox"
chmod +x "$CONTENTS/MacOS/Toolbox"

echo "==> Assembling bundle"
cp "$ROOT/Resources/AppIcon.icns" "$CONTENTS/Resources/AppIcon.icns"
sed -e "s/__VERSION__/$VERSION/" -e "s/__BUILD__/$BUILD_NUMBER/" \
  "$ROOT/Resources/Info.plist" > "$CONTENTS/Info.plist"
printf 'APPL????' > "$CONTENTS/PkgInfo"

# Resolve the signing identity.
IDENTITY=""
case "$SIGN_MODE" in
  adhoc) IDENTITY="-" ;;
  auto)
    IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null \
      | grep "Developer ID Application" | head -1 \
      | sed -E 's/.*"(.*)".*/\1/' || true)"
    if [[ -z "$IDENTITY" ]]; then
      echo "    No Developer ID Application certificate found — using ad-hoc."
      IDENTITY="-"
    fi
    ;;
  *) IDENTITY="$SIGN_MODE" ;;
esac

echo "==> Signing (${IDENTITY})"
# --options runtime enables the Hardened Runtime, which notarization requires.
# It is harmless for ad-hoc builds, so it is always on for consistency.
codesign --force --deep --options runtime --timestamp=none \
  --entitlements "$ROOT/Resources/Toolbox.entitlements" \
  --sign "$IDENTITY" "$APP"

echo "==> Verifying"
codesign --verify --deep --strict --verbose=1 "$APP"
lipo -archs "$CONTENTS/MacOS/Toolbox"

echo ""
echo "Built $APP (version $VERSION, build $BUILD_NUMBER)"
