#!/bin/sh
# Usage: doctor.sh  (run from anywhere inside the repo)
# Read-only. Exits non-zero with the failed check named.
set -eu
root="$(git rev-parse --show-toplevel)/ToolboxTauri"
app_path="$root/src-tauri/target/debug/bundle/macos/Toolbox Verification.app"
fail() { echo "doctor: FAIL $1" >&2; exit 1; }

pkg="$(node -p "require('$root/package.json').version")"
conf="$(node -p "require('$root/src-tauri/tauri.conf.json').version")"
[ "$pkg" = "$conf" ] || fail "package.json $pkg != tauri.conf.json $conf"
[ -d "$app_path" ] || fail "bundle missing at $app_path. Run Launch first."
bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app_path/Contents/Info.plist")"
[ "$bundle_id" = com.toolbox.desktop.verify ] || fail "bundle id is $bundle_id"
bundle_ver="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app_path/Contents/Info.plist")"
[ "$bundle_ver" = "$pkg" ] || fail "bundle version $bundle_ver != $pkg. Rebuild."
# lsof also lists AppKit XPC services (Open panel, theme widgets) that map the
# binary once a panel has opened. Keep only processes running the binary itself.
app_pids() {
  for p in $(lsof -t "$app_path/Contents/MacOS/toolbox" 2>/dev/null); do
    case "$(ps -o comm= -p "$p")" in
      */"Toolbox Verification.app/Contents/MacOS/toolbox") echo "$p" ;;
    esac
  done
}
pid="$(app_pids)"
[ -n "$pid" ] || fail "verification app is not running"
[ "$(echo "$pid" | wc -l | tr -d ' ')" = 1 ] || fail "more than one verification instance: $pid"
echo "doctor: OK version=$pkg pid=$pid"
