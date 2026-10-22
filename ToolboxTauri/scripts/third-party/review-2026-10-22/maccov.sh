#!/bin/bash
# Copies the macOS helper binaries plus their non-system dylib closure into $1.
set -eu
out="$1"; rm -rf "$out"; mkdir -p "$out"; seen="$out/.seen"; : > "$seen"
walk() {
  local f="$1" d d2 r n
  for d in $(otool -L "$f" | awk 'NR>1{print $1}'); do
    case "$d" in /usr/lib/*|/System/*) continue;; @loader_path/*) d2="$(dirname "$f")/${d#*/}";;
      @rpath/*) d2=""; for rp in $(otool -l "$f" | awk '/LC_RPATH/{x=1;next} x&&/path /{print $2;x=0}'); do rp="${rp/@loader_path/$(dirname "$f")}"; [ -f "$rp/${d#@rpath/}" ] && { d2="$rp/${d#@rpath/}"; break; }; done; [ -z "$d2" ] && d2="$(dirname "$f")/${d#*/}";; *) d2="$d";; esac
    r=$(realpath "$d2" 2>/dev/null || true); [ -z "$r" ] && { echo "unresolved $d from $f"; continue; }
    n=$(basename "$d2"); grep -qx "$n" "$seen" && continue; echo "$n" >> "$seen"
    cp -L "$r" "$out/$n"; walk "$r"
  done
}
for t in qpdf pdftoppm pdftotext tesseract; do p=$(command -v $t); cp -L "$p" "$out/"; walk "$(realpath "$p")"; done
rm "$seen"
