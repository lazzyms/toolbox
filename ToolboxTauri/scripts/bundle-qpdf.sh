#!/usr/bin/env bash
# Bundles a self-contained qpdf into a freshly built macOS Toolbox.app so
# Protect works out of the box. Run after `npm run tauri build`.
#
# Homebrew's qpdf is dynamically linked against libqpdf and (transitively)
# libjpeg/libcrypto. A bare copy would fail with "Library not loaded". Instead
# we copy qpdf plus its non-system dylib closure into Contents/Resources/qpdf-bin/
# and rebase every install name to @loader_path so the whole set travels inside
# the bundle with no host libraries required (macOS /usr/lib & /System are exempt).
set -euo pipefail

APP="${1:-$(pwd)/src-tauri/target/release/bundle/macos/Toolbox.app}"
QPDF_BIN="${QPDF:-$(command -v qpdf || true)}"

if [[ ! -d "$APP" ]]; then
  echo "bundle-qpdf: app bundle not found at $APP" >&2
  exit 1
fi
if [[ -z "$QPDF_BIN" ]]; then
  echo "bundle-qpdf: qpdf not found; install it (brew install qpdf) or set QPDF" >&2
  exit 1
fi

RES="$APP/Contents/Resources"
BIN_DIR="$RES/qpdf-bin"
mkdir -p "$BIN_DIR"
chmod -R u+w "$BIN_DIR"

QPDF_NAME="qpdf"
cp "$QPDF_BIN" "$BIN_DIR/$QPDF_NAME"
chmod +x "$BIN_DIR/$QPDF_NAME"

collect_deps() {
  otool -L "$1" 2>/dev/null | awk 'NR>1 { print $1 }' || true
}

collect_rpaths() {
  otool -l "$1" 2>/dev/null | awk '
    /LC_RPATH/ { in_rpath = 1; next }
    in_rpath && /path / { print $2; in_rpath = 0 }
  '
}

is_system_dep() {
  case "$1" in
    /usr/lib/*|/System/*) return 0 ;;
    *) return 1 ;;
  esac
}

resolve_dep() {
  local file="$1"
  local dep="$2"
  local candidate rpath

  case "$dep" in
    /*) [[ -f "$dep" ]] && printf '%s\n' "$dep" ;;
    @loader_path/*)
      candidate="$(dirname "$file")/${dep#@loader_path/}"
      [[ -f "$candidate" ]] && printf '%s\n' "$candidate"
      ;;
    @executable_path/*)
      candidate="$(dirname "$file")/${dep#@executable_path/}"
      [[ -f "$candidate" ]] && printf '%s\n' "$candidate"
      ;;
    @rpath/*)
      while IFS= read -r rpath; do
        case "$rpath" in
          @loader_path/*) rpath="$(dirname "$file")/${rpath#@loader_path/}" ;;
          @executable_path/*) rpath="$(dirname "$file")/${rpath#@executable_path/}" ;;
        esac
        candidate="$rpath/${dep#@rpath/}"
        if [[ -f "$candidate" ]]; then
          printf '%s\n' "$candidate"
          return
        fi
      done < <(collect_rpaths "$file")
      ;;
  esac
}

# Track which local dylibs have been pulled in, via a marker file (avoids the
# macOS bash 3.2 associative-array limitation).
handled="$BIN_DIR/.bundle-handled"
: > "$handled"

process_file() { # $1 = source file, $2 = copied file
  local source="$1"
  local file="$2"
  local dep resolved dep_name
  for dep in $(collect_deps "$source"); do
    is_system_dep "$dep" && continue
    resolved="$(resolve_dep "$source" "$dep")"
    if [[ -z "$resolved" ]]; then
      echo "bundle-qpdf: cannot resolve $dep from $source" >&2
      exit 1
    fi
    dep_name="$(basename "$resolved")"

    if ! grep -Fxq "$dep_name" "$handled"; then
      cp "$resolved" "$BIN_DIR/$dep_name"
      echo "$dep_name" >> "$handled"
      install_name_tool -id "@loader_path/$dep_name" "$BIN_DIR/$dep_name"
      process_file "$resolved" "$BIN_DIR/$dep_name"
    fi

    install_name_tool -change "$dep" "@loader_path/$dep_name" "$file"
  done
}

process_file "$QPDF_BIN" "$BIN_DIR/$QPDF_NAME"

# Sanity check: no unresolved non-system deps on the top-level binary.
while IFS= read -r dep; do
  is_system_dep "$dep" && continue
  case "$dep" in
    @loader_path/*) [[ -f "$BIN_DIR/${dep#@loader_path/}" ]] || { echo "bundle-qpdf: missing $dep" >&2; exit 1; } ;;
    *) echo "bundle-qpdf: unresolved dependency $dep" >&2; exit 1 ;;
  esac
done < <(collect_deps "$BIN_DIR/$QPDF_NAME")

rm -f "$handled"

# Sign the injected binaries with the app's identity or an explicit override.
if ! command -v codesign >/dev/null 2>&1; then
  echo "bundle-qpdf: codesign is required on macOS" >&2
  exit 1
fi
SIGNING_IDENTITY="${TOOLBOX_CODESIGN_IDENTITY:--}"
find "$BIN_DIR" -type f -exec codesign --force --sign "$SIGNING_IDENTITY" {} \;
codesign --force --sign "$SIGNING_IDENTITY" "$APP"
codesign --verify --deep --strict "$APP"

echo "bundle-qpdf: embedded self-contained qpdf -> $BIN_DIR"
VERSION="$("$BIN_DIR/$QPDF_NAME" --version 2>/dev/null | head -1)"
[[ -n "$VERSION" ]] || { echo "bundle-qpdf: qpdf --version failed" >&2; exit 1; }
echo "$VERSION"
