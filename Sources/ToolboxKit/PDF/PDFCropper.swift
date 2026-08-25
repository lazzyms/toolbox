import Foundation
import PDFKit

public struct PDFCropOptions: Sendable {
    public enum Mode: Sendable {
        /// Points shaved off each edge of the media box.
        case margins(top: Double, bottom: Double, left: Double, right: Double)
        /// A rectangle in PDF space (points, bottom-left origin) relative to
        /// the media box.
        case region(x: Double, y: Double, width: Double, height: Double)
    }

    public var mode: Mode

    public init(mode: Mode) {
        self.mode = mode
    }
}

public enum PDFCropper {
    public static func apply(
        _ options: PDFCropOptions,
        to input: URL,
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

        let croppedPages: Set<Int>
        if let rangeText = pageRangeText, !rangeText.trimmingCharacters(in: .whitespaces).isEmpty {
            croppedPages = Set(try PageRange.parse(rangeText, pageCount: pageCount))
        } else {
            croppedPages = Set(0..<pageCount)
        }

        for index in 0..<pageCount where croppedPages.contains(index) {
            guard let page = doc.page(at: index) else { continue }
            let box = try cropBox(for: page, mode: options.mode, input: input)
            page.setBounds(box, for: .cropBox)
        }

        let output = OutputNaming.destination(for: input, in: location, suffix: "-cropped", extension: "pdf")
        try PDFDocumentIO.save(doc, to: output)
        return output
    }

    private static func cropBox(for page: PDFPage, mode: PDFCropOptions.Mode, input: URL) throws -> CGRect {
        let media = page.bounds(for: .mediaBox)

        let candidate: CGRect
        switch mode {
        case .margins(let top, let bottom, let left, let right):
            // Validate the raw arithmetic — CGRect's width/height accessors
            // return standardized (absolute) values, which would hide a
            // negative result from the guard below.
            let width = media.width - left - right
            let height = media.height - top - bottom
            guard width > 0, height > 0 else {
                throw ToolboxError.invalidCropBox(
                    "The margins are larger than page “\(input.lastPathComponent)” — reduce the margins."
                )
            }
            candidate = CGRect(x: media.minX + left, y: media.minY + bottom, width: width, height: height)
        case .region(let x, let y, let width, let height):
            let requested = CGRect(
                x: media.minX + x, y: media.minY + y,
                width: max(0, width), height: max(0, height)
            )
            candidate = requested.intersection(media)
        }

        // A crop box only hides content outside it — but a box that inverts or
        // zeroes the page leaves nothing to show, so reject it outright.
        guard !candidate.isNull, !candidate.isEmpty else {
            throw ToolboxError.invalidCropBox(
                "The crop region doesn't overlap page “\(input.lastPathComponent)”."
            )
        }
        return candidate
    }
}
