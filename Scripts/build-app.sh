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
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources" "$CONTENTS/Frameworks"

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

echo "==> Embedding Sparkle.framework"
# SwiftPM links the XCFramework but does not embed it, so a hand-assembled
# bundle has to copy the right slice in itself or the app won't launch.
SPARKLE_XC="$(find "$ROOT/.build/artifacts" -type d -name "Sparkle.framework" \
  -path "*macos-arm64_x86_64*" 2>/dev/null | head -1)"
if [[ -z "$SPARKLE_XC" ]]; then
  echo "error: Sparkle.framework not found — run 'swift package resolve' first" >&2
  exit 1
fi
# -R preserves symlinks and the internal Versions/ layout that frameworks need.
cp -R "$SPARKLE_XC" "$CONTENTS/Frameworks/Sparkle.framework"

# The linker records an @rpath reference; without this the dylib isn't found.
install_name_tool -add_rpath "@executable_path/../Frameworks" \
  "$CONTENTS/MacOS/Toolbox" 2>/dev/null || true

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
SIGN_FLAGS=(--force --options runtime --timestamp=none --sign "$IDENTITY")

# Sign nested code inside-out. Apple's --deep is explicitly documented as
# unsuitable for this and silently mis-signs Sparkle's helpers, which then get
# rejected at update time — so each component is signed explicitly, deepest
# first. Order matters: a parent's seal covers its children's signatures.
SPARKLE_FW="$CONTENTS/Frameworks/Sparkle.framework"
SPARKLE_V="$SPARKLE_FW/Versions/B"

if [[ "$IDENTITY" == "-" ]]; then
  # Ad-hoc builds must disable Library Validation, which otherwise requires the
  # framework to share the app's Team ID — impossible when neither has one:
  #   "mapping process and mapped file (non-platform) have different Team IDs"
  # See Resources/Toolbox-adhoc.entitlements for the security tradeoff.
  echo "    ad-hoc: disabling library validation so Sparkle.framework can load"
  codesign "${SIGN_FLAGS[@]}" --deep \
    --entitlements "$ROOT/Resources/Toolbox-adhoc.entitlements" "$APP"
else
  # With a real Developer ID every signature carries the same Team ID, so the
  # correct inside-out order works and is preferred: --deep is documented as
  # unsuitable here and mis-signs Sparkle's helpers, which then get rejected
  # during an update.
  for HELPER in \
    "$SPARKLE_V/XPCServices/Downloader.xpc" \
    "$SPARKLE_V/XPCServices/Installer.xpc" \
    "$SPARKLE_V/Updater.app/Contents/MacOS/Updater" \
    "$SPARKLE_V/Updater.app" \
    "$SPARKLE_V/Autoupdate" \
    "$SPARKLE_V/Sparkle"
  do
    [[ -e "$HELPER" ]] && codesign "${SIGN_FLAGS[@]}" "$HELPER"
  done

  # The framework wrapper, then the app. Only the app gets entitlements —
  # applying them to helpers would break Sparkle's privilege separation.
  codesign "${SIGN_FLAGS[@]}" "$SPARKLE_FW"
  codesign "${SIGN_FLAGS[@]}" \
    --entitlements "$ROOT/Resources/Toolbox.entitlements" "$APP"
fi

echo "==> Verifying"
codesign --verify --deep --strict --verbose=1 "$APP"
# Confirm the framework is embedded and reachable, since a missing rpath only
# shows up as a launch crash otherwise.
otool -l "$CONTENTS/MacOS/Toolbox" | grep -A2 LC_RPATH | grep -q "Frameworks" \
  && echo "    rpath to Frameworks present"
test -x "$SPARKLE_V/Sparkle" && echo "    Sparkle.framework embedded"
lipo -archs "$CONTENTS/MacOS/Toolbox"

# Actually launch it. A signing or embedding mistake shows up only at runtime as
# a dyld failure, and shipping a bundle that dies on launch is the worst
# possible outcome — so verify rather than trust the signature checks above.
echo "==> Smoke-testing launch"
"$CONTENTS/MacOS/Toolbox" >/tmp/toolbox-launch.log 2>&1 &
LAUNCH_PID=$!
sleep 4
if kill -0 "$LAUNCH_PID" 2>/dev/null; then
  kill "$LAUNCH_PID" 2>/dev/null || true
  wait "$LAUNCH_PID" 2>/dev/null || true
  echo "    launches cleanly with Sparkle loaded"
else
  echo "" >&2
  echo "error: the app failed to launch. Output:" >&2
  sed 's/^/    /' /tmp/toolbox-launch.log >&2
  exit 1
fi

echo ""
echo "Built $APP (version $VERSION, build $BUILD_NUMBER)"
