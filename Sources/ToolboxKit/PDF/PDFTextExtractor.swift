import Foundation
import AppKit
import PDFKit

public enum PDFTextExportStyle: Sendable {
    case plainText
    case markdown
}

public struct PDFTextOptions: Sendable {
    public var style: PDFTextExportStyle
    /// Inserts `--- page N ---` between pages.
    public var includePageSeparators: Bool

    public init(style: PDFTextExportStyle = .plainText, includePageSeparators: Bool = false) {
        self.style = style
        self.includePageSeparators = includePageSeparators
    }
}

public enum PDFTextExtractor {
    public static func extract(
        _ input: URL,
        options: PDFTextOptions,
        pageRangeText: String?,
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

        let selected: [Int]
        if let rangeText = pageRangeText, !rangeText.trimmingCharacters(in: .whitespaces).isEmpty {
            selected = try PageRange.parse(rangeText, pageCount: pageCount)
        } else {
            selected = Array(0..<pageCount)
        }

        var pages: [String] = []
        for index in selected {
            guard let page = doc.page(at: index) else { continue }
            switch options.style {
            case .plainText:
                pages.append(page.string ?? "")
            case .markdown:
                pages.append(markdown(for: page))
            }
        }

        // The single most likely support question: a scanned PDF has no text
        // layer, and an empty file presented as success would be a lie.
        guard pages.contains(where: { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) else {
            throw ToolboxError.noTextLayer(input)
        }

        var text = ""
        for (position, pageText) in pages.enumerated() {
            let pageNumber = selected[position] + 1
            if options.includePageSeparators, position > 0 {
                text += "\n--- page \(pageNumber) ---\n\n"
            }
            text += pageText
            if !text.hasSuffix("\n") {
                text += "\n"
            }
        }

        let ext = options.style == .markdown ? "md" : "txt"
        let output = OutputNaming.destination(
            for: input, in: location, suffix: "-text", extension: ext
        )
        try text.write(to: output, atomically: true, encoding: .utf8)
        return output
    }

    /// Best-effort markdown: blocks separated where the text has line breaks,
    /// headings guessed from relative font size. Layout analysis this is not —
    /// the UI says so plainly.
    private static func markdown(for page: PDFPage) -> String {
        guard let attributed = page.attributedString,
              attributed.length > 0 else {
            return ""
        }

        // The body size is the most common run size — headings are rare, so
        // ties break toward the smaller size. A median would misfire on
        // title+body documents where the two sizes are equally represented.
        var sizes: [CGFloat] = []
        let full = NSRange(location: 0, length: attributed.length)
        attributed.enumerateAttribute(.font, in: full) { value, _, _ in
            if let font = value as? NSFont {
                sizes.append(font.pointSize)
            }
        }
        guard !sizes.isEmpty else {
            return attributed.string
        }
        var counts: [CGFloat: Int] = [:]
        for size in sizes {
            counts[size, default: 0] += 1
        }
        let bodySize = counts.min { lhs, rhs in
            lhs.value != rhs.value ? lhs.value > rhs.value : lhs.key < rhs.key
        }?.key ?? sizes.min() ?? 0

        var output: [String] = []
        var currentLine = ""
        var currentSize: CGFloat = 0

        func flush() {
            let trimmed = currentLine.trimmingCharacters(in: .whitespaces)
            currentLine = ""
            guard !trimmed.isEmpty else { return }
            if bodySize > 0, currentSize >= bodySize * 1.35 {
                output.append("# \(trimmed)")
            } else {
                output.append(trimmed)
            }
            currentSize = 0
        }

        attributed.enumerateAttributes(in: full) { attributes, range, _ in
            let chunk = attributed.attributedSubstring(from: range).string
            let size = (attributes[.font] as? NSFont)?.pointSize ?? bodySize

            for piece in chunk.components(separatedBy: "\n") {
                if piece.isEmpty {
                    flush()
                    continue
                }
                if currentSize != 0 && abs(size - currentSize) > 0.5 {
                    flush()
                }
                if currentSize == 0 {
                    currentSize = size
                }
                currentLine += piece
            }
        }
        flush()

        return output.joined(separator: "\n\n")
    }
}
