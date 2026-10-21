#!/bin/sh
# Usage: make-fixtures.sh <run-dir>
# Writes disposable inputs to <run-dir>/scratch/ and prints their absolute paths.
set -eu
run_dir="${1:?usage: make-fixtures.sh <run-dir>}"
mkdir -p "$run_dir/scratch"
scratch="$(cd "$run_dir/scratch" && pwd)"
command -v qpdf >/dev/null || { echo "qpdf not found on PATH" >&2; exit 1; }

raw="$scratch/.raw.pdf"
cat > "$raw" <<'PDF'
%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj
4 0 obj << /Length 52 >> stream
BT /F1 24 Tf 72 700 Td (Toolbox verification) Tj ET
endstream endobj
5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
trailer << /Size 6 /Root 1 0 R >>
%%EOF
PDF
qpdf "$raw" "$scratch/plain.pdf" 2>/dev/null || [ $? -eq 3 ]
rm -f "$raw"

owner_pw="$(openssl rand -hex 12)"
qpdf --encrypt --user-password= --owner-password="$owner_pw" --bits=256 \
  --print=none --modify=none --extract=n -- "$scratch/plain.pdf" "$scratch/restricted.pdf"

printf 'Legacy Word stub for Toolbox verification\n' > "$scratch/legacy.doc"

(cd "$scratch" && shasum -a 256 plain.pdf restricted.pdf legacy.doc > inputs.sha256)
qpdf --show-encryption "$scratch/restricted.pdf" | head -3 >&2
ls -1 "$scratch"/plain.pdf "$scratch"/restricted.pdf "$scratch"/legacy.doc
