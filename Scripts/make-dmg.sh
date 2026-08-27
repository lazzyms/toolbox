#!/bin/bash
#
# Packages dist/Toolbox.app into an installable DMG with an Applications alias.
#
#   ./Scripts/make-dmg.sh [--version 1.0.0]
#
# Uses only built-in tools (hdiutil, osascript) — no Homebrew dependency, so it
# works on a clean machine.
#
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

VERSION="1.0.0"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    *) echo "unknown option: $1" >&2; exit 1 ;;
  esac
done

APP="$ROOT/dist/Toolbox.app"
DMG="$ROOT/dist/Toolbox-$VERSION.dmg"
STAGE="$ROOT/dist/dmg-stage"
VOLUME_NAME="Toolbox $VERSION"

if [[ ! -d "$APP" ]]; then
  echo "error: $APP not found — run ./Scripts/build-app.sh first" >&2
  exit 1
fi

echo "==> Staging"
rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
# -R preserves the signature; plain cp -r can break code signatures.
cp -R "$APP" "$STAGE/Toolbox.app"
ln -s /Applications "$STAGE/Applications"
mkdir -p "$STAGE/.background"
swift "$ROOT/Scripts/make-dmg-background.swift" "$STAGE/.background/Toolbox-install.png"

# A short README in the image covers the first-open step for unsigned builds.
cat > "$STAGE/Read Me First.txt" <<'TXT'
Toolbox — local file utilities for macOS

INSTALL
  Drag Toolbox to the Applications folder alongside this file.

FIRST LAUNCH
  macOS may block the first launch because this build is not notarized by Apple.
  To allow it, open:

    System Settings → Privacy & Security

  Scroll down to the "Toolbox" message and click "Open Anyway". Then confirm
  the prompt. You only need to do this once; later launches are normal.

WHAT IT DOES
  • Remove PDF Password — save an unlocked copy of a PDF you have the
    password for
  • Convert Image Format — HEIC/PNG/JPEG/TIFF, in batches
  • Compress Images — lossless, or lossy with a quality slider
  • Resize Images — by box, longest side, percentage or exact pixels

PRIVACY
  Everything runs on your Mac. The app has no network code and never
  uploads your files.
TXT

echo "==> Creating read-write image"
TEMP_DMG="$ROOT/dist/temp.dmg"
rm -f "$TEMP_DMG"
hdiutil create -srcfolder "$STAGE" -volname "$VOLUME_NAME" \
  -fs HFS+ -format UDRW -ov "$TEMP_DMG" >/dev/null

echo "==> Arranging window"
MOUNT_DIR="$(mktemp -d)"
hdiutil attach "$TEMP_DMG" -mountpoint "$MOUNT_DIR" -nobrowse -noautoopen >/dev/null

# Best-effort cosmetics. A sandboxed or headless shell can't drive Finder, and
# an ugly-but-working DMG beats a failed build, so never abort on this.
osascript >/dev/null 2>&1 <<OSA || echo "    (skipped Finder layout — not fatal)"
tell application "Finder"
  tell disk "$VOLUME_NAME"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set the bounds of container window to {200, 150, 800, 570}
    set opts to the icon view options of container window
    set arrangement of opts to not arranged
    set icon size of opts to 128
    set background picture of opts to file ".background:Toolbox-install.png"
    set position of item "Toolbox.app" of container window to {150, 190}
    set position of item "Applications" of container window to {450, 190}
    set position of item "Read Me First.txt" of container window to {300, 340}
    close
    open
    update without registering applications
    delay 1
  end tell
end tell
OSA

sync
hdiutil detach "$MOUNT_DIR" -force >/dev/null 2>&1 || true
rmdir "$MOUNT_DIR" 2>/dev/null || true

echo "==> Compressing"
# UDZO with maximum zlib compression keeps the download small.
hdiutil convert "$TEMP_DMG" -format UDZO -imagekey zlib-level=9 -o "$DMG" >/dev/null
rm -f "$TEMP_DMG"
rm -rf "$STAGE"

# Sign the DMG itself when a Developer ID is available; harmless to skip.
IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null \
  | grep "Developer ID Application" | head -1 | sed -E 's/.*"(.*)".*/\1/' || true)"
if [[ -n "$IDENTITY" ]]; then
  echo "==> Signing DMG"
  codesign --force --sign "$IDENTITY" "$DMG"
fi

echo "==> Verifying"
hdiutil verify "$DMG" >/dev/null && echo "    image is valid"
SIZE="$(du -h "$DMG" | cut -f1 | tr -d ' ')"

echo ""
echo "Created $DMG ($SIZE)"
echo "SHA-256: $(shasum -a 256 "$DMG" | cut -d' ' -f1)"
