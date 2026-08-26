import Foundation
import PDFKit

public struct PDFPageRemoveOptions: Sendable {
    /// The user-entered pages to delete, e.g. "2, 5-7, 11-".
    public var pages: String

    public init(pages: String) {
        self.pages = pages
    }
}

public enum PDFPageRemover {
    /// Writes a copy of `input` with the selected pages deleted, survivors in
    /// their original order.
    public static func remove(
        _ input: URL,
        options: PDFPageRemoveOptions,
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

        // Parsed against this document's own length, so a batch member shorter
        // than the selection fails loudly instead of quietly dropping less.
        let selected = Set(try PageRange.parse(options.pages, pageCount: pageCount))
        guard selected.count < pageCount else {
            throw ToolboxError.removesAllPages(input)
        }

        // Keep-and-copy beats removePage(at:) in a loop: deleting shifts the
        // indices underneath the iteration, and a fresh document also can't
        // inherit encryption state from the source file.
        let trimmed = PDFDocument()
        for index in 0..<pageCount where !selected.contains(index) {
            guard let page = doc.page(at: index)?.copy() as? PDFPage else { continue }
            trimmed.insert(page, at: trimmed.pageCount)
        }

        let output = OutputNaming.destination(
            for: input,
            in: location,
            suffix: "-trimmed",
            extension: "pdf"
        )
        try PDFDocumentIO.save(trimmed, to: output)
        return output
    }
}
