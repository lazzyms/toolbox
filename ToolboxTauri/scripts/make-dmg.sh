#!/usr/bin/env bash
set -euo pipefail

APP="${1:?app bundle path is required}"
DMG="${2:?dmg output path is required}"

if [[ ! -d "$APP" ]]; then
  echo "make-dmg: app bundle not found at $APP" >&2
  exit 1
fi

STAGING="$(mktemp -d -t toolbox-dmg)"
mkdir -p "$(dirname "$DMG")"
ditto "$APP" "$STAGING/Toolbox.app"
ln -s /Applications "$STAGING/Applications"
hdiutil create -ov -format UDZO -volname Toolbox -srcfolder "$STAGING" "$DMG"

echo "make-dmg: created $DMG"
