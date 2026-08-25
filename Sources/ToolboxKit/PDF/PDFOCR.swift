import Foundation
import CoreGraphics
import PDFKit
import Vision

public struct PDFOCROptions: Sendable {
    /// Render resolution fed to Vision — higher costs time linearly.
    public var dpi: Int
    /// Inserts `--- page N ---` between pages, matching the text extractor.
    public var includePageSeparators: Bool

    public init(dpi: Int = 150, includePageSeparators: Bool = false) {
        self.dpi = dpi
        self.includePageSeparators = includePageSeparators
    }
}

/// Reads text out of scanned PDFs by recognising rendered pixels.
///
/// Unlike `PDFTextExtractor`, which reads the text layer and refuses scans,
/// this renders each selected page to a bitmap and runs Apple's on-device
/// Vision OCR over it. A scan with nothing legible produces an empty file —
/// that is a legitimate outcome, not an error.
public enum PDFOCR {
    public static func recognize(
        _ input: URL,
        options: PDFOCROptions,
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

        let scale = CGFloat(options.dpi) / 72
        var pages: [[String]] = []
        for index in selected {
            guard let page = doc.page(at: index) else { continue }
            let media = page.bounds(for: .mediaBox)
            let width = Int((media.width * scale).rounded())
            let height = Int((media.height * scale).rounded())
            guard width > 0, height > 0 else { continue }
            guard width <= PDFImageExporter.maxDimension,
                  height <= PDFImageExporter.maxDimension,
                  width * height <= PDFImageExporter.maxPixels else {
                throw ToolboxError.resolutionTooLarge(width, height)
            }
            let image = try render(page: page, pixelWidth: width, pixelHeight: height, scale: scale)
            pages.append(try recognize(image))
        }

        var text = ""
        for (position, pageLines) in pages.enumerated() {
            let pageNumber = selected[position] + 1
            if options.includePageSeparators, position > 0 {
                text += "\n--- page \(pageNumber) ---\n\n"
            }
            text += pageLines.joined(separator: "\n")
            if !text.hasSuffix("\n") {
                text += "\n"
            }
        }

        let output = OutputNaming.destination(
            for: input, in: location, suffix: "-ocr-text", extension: "txt"
        )
        try text.write(to: output, atomically: true, encoding: .utf8)
        return output
    }

    /// Renders one page exactly the way PDF→Images does: white ground, DPI
    /// scale, vector replay. Sharing the constants keeps the memory guard in
    /// one place; the render itself stays local because Vision wants its
    /// bitmap handed straight back.
    private static func render(page: PDFPage, pixelWidth: Int, pixelHeight: Int, scale: CGFloat) throws -> CGImage {
        guard let context = CGContext(
            data: nil,
            width: pixelWidth,
            height: pixelHeight,
            bitsPerComponent: 8,
            bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else {
            throw ToolboxError.encodeFailed("PDF page")
        }

        // White ground: transparent page areas must land on white or Vision
        // reads noise out of the black that would otherwise show through.
        context.setFillColor(CGColor(gray: 1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: pixelWidth, height: pixelHeight))

        context.scaleBy(x: scale, y: scale)
        PDFPageReplay.replay(page: page, into: context)

        guard let image = context.makeImage() else {
            throw ToolboxError.encodeFailed("PDF page")
        }
        return image
    }

    /// Runs Vision over one bitmap and returns recognised text ordered
    /// top-to-bottom, left-to-right, one string per visual line.
    ///
    /// `perform` blocks until recognition finishes, so this whole function is
    /// synchronous despite Vision's asynchronous-looking request API.
    private static func recognize(_ image: CGImage) throws -> [String] {
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.recognitionLanguages = ["en-US"]

        let handler = VNImageRequestHandler(cgImage: image)
        do {
            try handler.perform([request])
        } catch {
            throw ToolboxError.ocrFailed(error.localizedDescription)
        }

        guard let observations = request.results else { return [] }
        var fragments: [(text: String, box: CGRect)] = []
        for observation in observations {
            guard let candidate = observation.topCandidates(1).first else { continue }
            fragments.append((candidate.string, observation.boundingBox))
        }

        // Vision boxes are normalised with a bottom-left origin, so a larger
        // midY sits higher on the page. Fragments whose midpoints sit within
        // half the smaller glyph height belong to the same visual line.
        func sameLine(_ lhs: CGRect, _ rhs: CGRect) -> Bool {
            abs(lhs.midY - rhs.midY) <= 0.5 * min(lhs.height, rhs.height)
        }
        fragments.sort { lhs, rhs in
            if !sameLine(lhs.box, rhs.box) {
                return lhs.box.midY > rhs.box.midY
            }
            return lhs.box.minX < rhs.box.minX
        }

        var lines: [String] = []
        var current: [String] = []
        var anchor = CGRect.null
        for fragment in fragments {
            if !current.isEmpty && sameLine(anchor, fragment.box) {
                current.append(fragment.text)
            } else {
                if !current.isEmpty {
                    lines.append(current.joined(separator: " "))
                }
                current = [fragment.text]
                anchor = fragment.box
            }
        }
        if !current.isEmpty {
            lines.append(current.joined(separator: " "))
        }
        return lines
    }
}
