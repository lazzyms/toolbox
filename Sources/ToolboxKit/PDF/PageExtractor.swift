import Foundation
import PDFKit

public enum PageExtractor {
    /// Writes the selected pages of `input` into one new PDF.
    ///
    /// `selection` uses the same syntax as everywhere else — "1-3, 7, 9-",
    /// `odd` / `even` — and is resolved by `PageRange.parse`, whose stance
    /// mirrors Split's: overlapping or repeated picks collapse, so every
    /// selected page appears exactly once, in ascending page order.
    ///
    /// An empty selection is rejected rather than meaning "all pages":
    /// silently copying a whole document under a "-pages" name would read as
    /// an extraction that never happened.
    public static func extract(
        _ input: URL,
        selection: String,
        to location: OutputLocation
    ) throws -> URL {
        if PDFUnlocker.isEncrypted(input) {
            throw ToolboxError.passwordProtected(input)
        }

        let doc = try PDFDocumentIO.open(input)
        let pageCount = doc.pageCount
        guard pageCount > 0 else {
            throw ToolboxError.notAPDF(input)
        }

        // Same normalisation as PageRange.parse, run first because parse
        // treats blank input as "every page" — here that must be an error.
        let tokens = selection.split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        guard !tokens.isEmpty else {
            throw ToolboxError.invalidPageRange(
                "Select at least one page — e.g. “1-3, 7”."
            )
        }

        let pages = try PageRange.parse(selection, pageCount: pageCount)

        // A fresh document per output, as in merge and split: writing pages
        // copied straight from the source risks dragging along state — and
        // any encryption dictionary — from the file it came from.
        let extracted = PDFDocument()
        for index in pages {
            guard let page = doc.page(at: index)?.copy() as? PDFPage else { continue }
            extracted.insert(page, at: extracted.pageCount)
        }

        let output = OutputNaming.destination(
            for: input,
            in: location,
            suffix: "-pages",
            extension: "pdf"
        )
        try PDFDocumentIO.save(extracted, to: output)
        return output
    }
}
