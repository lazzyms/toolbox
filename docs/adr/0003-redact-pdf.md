# ADR 0003: Redaction that actually removes

Status: proposed · Design for #23 (implementation to follow)

## Context

The Crop PDF tool documents that a crop box only *hides* content. Redaction
exists because hiding is not enough: black rectangles drawn over a page leave
the underlying text fully extractable, which is the classic redaction failure.
A credible Redact PDF tool must remove the covered content from the file.

## Options considered

1. **Vector overlay.** Replay pages and paint opaque rects over them. The text
   under the rects survives in the new content stream — `pdftotext` reads it
   right back. Rejected: false security is worse than none.
2. **Rasterise only the redacted pages.** Render each affected page to a
   bitmap (the `PDFImageExporter` render path), paint the rects onto the
   bitmap, embed it as an image page; untouched pages stay vector. Pixels have
   no text layer, so the covered words are gone from the output entirely.
3. **Content-stream surgery.** Excise the specific text runs under each rect.
   Correct in theory; in practice TJ arrays, subset encodings and reflow make
   this extremely error-prone for a v1.

## Decision

Option 2, with plain disclosure in the UI: pages you redact become images at
~200 dpi — selectable text on those pages is gone along with the secret;
untouched pages keep theirs. Rectangles come from a region picker per document,
applied via the existing page-range support. Originals stay untouched and
outputs route through `-redacted` naming as everywhere else.

## Consequences

True removal at the cost of rasterised pages, which is the trade every honest
offline redaction tool makes. File size grows on affected pages. A later
enhancement can attempt option 3 for text-only runs, but the default must never
regress to hiding.
