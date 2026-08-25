# ADR 0001: Repairing damaged PDFs

Status: proposed · Closes #24

## Context

`PDFDocumentIO.open` fails on PDFs with a damaged or missing cross-reference
table — the most common corruption from interrupted downloads and old
generators. Stock macOS has no Ghostscript or qpdf to lean on, and ToolboxKit
stays dependency-free, so any repair is our own code.

## Options considered

1. **PDFKit leniency first.** `PDFDocument(url:)` sometimes opens files that
   CGPDF rejects; re-saving through it rebuilds a clean structure for free.
   Cheap, worth trying first, but not universal.
2. **Full xref reconstruction.** Scan the byte stream for `N G obj … endobj`,
   index every object, rebuild the table and trailer in pure Swift (~300
   lines). Handles the worst cases, including no trailer at all.
3. **Rasterise as a fallback.** Always works, always destroys selectable text.
   Rejected as a *repair*: it fails the invariant the unlock tool already
   enforces.

## Decision

Ship one entry point, `PDFRepairer.repair(_:to:)`, that tries option 1 then
falls back to option 2, verifies by reopening the output, and routes through
`OutputNaming` with a `-repaired` suffix. Option 2 is real parser work with a
fuzz-testing burden — it should be its own issue with its own test corpus of
broken files, not a side effect of another change.

## Consequences

A repair path exists for the long tail without new dependencies. The scanner
needs careful bounds-checking (untrusted input), so it lands behind tests built
from deliberately corrupted fixtures before the tool surfaces in the registry.
