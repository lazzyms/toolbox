import Foundation

/// Parses user-entered page selections like "1-3, 7, 9-" into 0-based indices.
///
/// The user thinks in 1-based page numbers; `PDFDocument` and page arrays are
/// 0-based. This is the single place that boundary is crossed, so the
/// off-by-one bugs in the PDF tools all have exactly one home.
public enum PageRange {

    /// Parses `text` against a document of `pageCount` pages.
    ///
    /// Accepted forms, comma-separated and case-insensitive:
    /// - `7` — a single page
    /// - `1-3` — an inclusive range (reversed input like `7-3` still works)
    /// - `9-` — page 9 through to the end
    /// - `-3` — page 1 through to page 3
    /// - `odd` / `even` — every odd- or even-numbered page
    ///
    /// Empty (or whitespace-only) text means every page. Duplicates and
    /// overlaps collapse and the result is sorted ascending. Zero,
    /// out-of-bounds pages and unparseable tokens throw
    /// `ToolboxError.invalidPageRange` rather than being silently clamped, so
    /// a typo like `1-999` can't quietly drop pages.
    public static func parse(_ text: String, pageCount: Int) throws -> [Int] {
        let tokens = text.split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }

        guard !tokens.isEmpty else {
            // "All pages" — which for an empty document is nothing.
            return Array(0..<max(pageCount, 0))
        }
        guard pageCount >= 1 else {
            throw ToolboxError.invalidPageRange(
                "This document has no pages to select from."
            )
        }

        var pages = Set<Int>()
        for token in tokens {
            try add(token, pageCount: pageCount, into: &pages)
        }
        return pages.sorted()
    }

    private static func add(_ token: String, pageCount: Int, into pages: inout Set<Int>) throws {
        let normalized = token.trimmingCharacters(in: .whitespaces).lowercased()

        if normalized == "odd" {
            for page in stride(from: 1, through: pageCount, by: 2) {
                pages.insert(page - 1)
            }
            return
        }
        if normalized == "even" {
            for page in stride(from: 2, through: pageCount, by: 2) {
                pages.insert(page - 1)
            }
            return
        }

        let parts = normalized.split(separator: "-", omittingEmptySubsequences: false)
            .map { $0.trimmingCharacters(in: .whitespaces) }

        if parts.count == 1 {
            let page = try page(from: parts[0])
            try requireInBounds(page, pageCount: pageCount)
            pages.insert(page - 1)
        } else if parts.count == 2 {
            // A bare dash means nothing on either side — reject it with a
            // message that shows the token, not empty quotes.
            if parts[0].isEmpty && parts[1].isEmpty {
                throw ToolboxError.invalidPageRange("“\(normalized)” isn't a valid page range.")
            }
            if parts[0].isEmpty {
                // "-B" — from page 1 through to B.
                let last = try page(from: parts[1])
                try requireInBounds(last, pageCount: pageCount)
                pages.formUnion(0...(last - 1))
            } else {
                let first = try page(from: parts[0])
                let last = parts[1].isEmpty ? pageCount : try page(from: parts[1])
                try requireInBounds(first, pageCount: pageCount)
                try requireInBounds(last, pageCount: pageCount)
                // Reversed input ("7-3") is just the range between the two.
                let lower = min(first, last) - 1
                let upper = max(first, last) - 1
                pages.formUnion(lower...upper)
            }
        } else {
            throw ToolboxError.invalidPageRange("“\(normalized)” isn't a valid page range.")
        }
    }

    /// Parses one endpoint: a whole number, 1-based, so 0 is rejected here.
    private static func page(from raw: String) throws -> Int {
        guard let number = Int(raw), number >= 1 else {
            throw ToolboxError.invalidPageRange(
                "“\(raw)” isn't a valid page number — pages are numbered from 1."
            )
        }
        return number
    }

    private static func requireInBounds(_ page: Int, pageCount: Int) throws {
        guard page <= pageCount else {
            throw ToolboxError.invalidPageRange(
                "Page \(page) is outside this \(pageCount)-page document."
            )
        }
    }
}
