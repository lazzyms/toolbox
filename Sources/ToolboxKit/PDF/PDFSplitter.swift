import Foundation
import PDFKit

public struct PDFSplitOptions: Sendable {
    public enum Mode: Sendable {
        /// One file per page.
        case everyPage
        /// One file per comma-separated range, e.g. "1-3, 4-8, 9-".
        case ranges(String)
        /// Fixed-size chunks; the last one may be short.
        case everyN(Int)
    }

    public var mode: Mode

    public init(mode: Mode) {
        self.mode = mode
    }
}

public enum PDFSplitter {
    public static func split(
        _ input: URL,
        options: PDFSplitOptions,
        to location: OutputLocation
    ) throws -> [URL] {
        if PDFUnlocker.isEncrypted(input) {
            throw ToolboxError.passwordProtected(input)
        }

        let doc = try PDFDocumentIO.open(input)
        let pageCount = doc.pageCount
        guard pageCount > 0 else {
            throw ToolboxError.notAPDF(input)
        }

        let chunks = try self.chunks(pageCount: pageCount, mode: options.mode)

        let base = input.deletingPathExtension().lastPathComponent
        let dir = location.directory(forInput: input)
        var outputs: [URL] = []

        for (offset, pages) in chunks.enumerated() {
            // A fresh document per chunk, as in merge: writing pages copied
            // straight into a reused document risks dragging along state from
            // the source file, and each output must stand alone.
            let chunk = PDFDocument()
            for index in pages {
                guard let page = doc.page(at: index)?.copy() as? PDFPage else { continue }
                chunk.insert(page, at: chunk.pageCount)
            }
            guard chunk.pageCount > 0 else { continue }

            // Numbering runs over chunks, not pages, so every mode reads
            // doc-1, doc-2, … regardless of how big each piece is.
            let stem = "\(base)-\(offset + 1)"
            let output = OutputNaming.destination(
                for: dir.appendingPathComponent(stem),
                in: location,
                extension: "pdf"
            )
            try PDFDocumentIO.save(chunk, to: output)
            outputs.append(output)
        }

        return outputs
    }

    private static func chunks(pageCount: Int, mode: PDFSplitOptions.Mode) throws -> [[Int]] {
        switch mode {
        case .everyPage:
            return (0..<pageCount).map { [$0] }
        case .everyN(let size):
            guard size >= 1 else {
                throw ToolboxError.invalidSplit("Chunk size must be at least one page.")
            }
            var groups: [[Int]] = []
            var start = 0
            while start < pageCount {
                groups.append(Array(start..<min(start + size, pageCount)))
                start += size
            }
            return groups
        case .ranges(let text):
            let tokens = text.split(separator: ",")
                .map { $0.trimmingCharacters(in: .whitespaces) }
                .filter { !$0.isEmpty }
            guard !tokens.isEmpty else {
                throw ToolboxError.invalidSplit(
                    "Enter at least one page range — e.g. “1-3, 4-8, 9-”."
                )
            }
            // Parsed one token at a time so each range stays a separate file;
            // a single parse of the whole string would collapse them into one
            // sorted selection.
            return try tokens.map { try PageRange.parse($0, pageCount: pageCount) }
        }
    }
}
