import Foundation
import CoreGraphics
import ImageIO
import PDFKit
import UniformTypeIdentifiers

public struct PDFToImagesOptions: Sendable {
    public var format: ImageFormat
    /// JPEG quality, 0–1. Ignored for lossless formats.
    public var quality: Double
    public var dpi: Int

    public init(format: ImageFormat = .jpeg, quality: Double = 0.85, dpi: Int = 150) {
        self.format = format
        self.quality = quality
        self.dpi = dpi
    }
}

/// Renders PDF pages to raster images at a chosen DPI. This is a
/// rasterisation — the text becomes pixels — which is exactly what the tool
/// promises and why #23 (redaction) is a different operation.
public enum PDFImageExporter {
    /// Guards against a poster-sized page at high DPI producing gigapixel
    /// bitmaps that exhaust memory.
    static let maxDimension = 16_000
    static let maxPixels = 80_000_000

    public static func convert(
        _ input: URL,
        options: PDFToImagesOptions,
        pageRangeText: String?,
        to location: OutputLocation
    ) throws -> [URL] {
        if PDFUnlocker.isEncrypted(input) {
            throw ToolboxError.passwordProtected(input)
        }
        guard options.format.canEncode else {
            throw ToolboxError.unsupportedOutput(options.format.displayName)
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
        let dir = location.directory(forInput: input)
        var outputs: [URL] = []

        for index in selected {
            guard let page = doc.page(at: index) else { continue }
            let media = page.bounds(for: .mediaBox)
            let width = Int((media.width * scale).rounded())
            let height = Int((media.height * scale).rounded())
            guard width > 0, height > 0 else { continue }
            guard width <= maxDimension, height <= maxDimension, width * height <= maxPixels else {
                throw ToolboxError.resolutionTooLarge(width, height)
            }

            let image = try render(page: page, pixelWidth: width, pixelHeight: height, scale: scale)
            // The stem carries the document's own page number so names stay
            // stable regardless of any range selection.
            let stem = "\(input.deletingPathExtension().lastPathComponent)-page-\(index + 1)"
            let output = OutputNaming.destination(
                for: dir.appendingPathComponent(stem),
                in: location,
                extension: options.format.fileExtension
            )
            try encode(image, to: output, format: options.format, quality: options.quality)
            outputs.append(output)
        }

        guard !outputs.isEmpty else {
            throw ToolboxError.notAPDF(input)
        }
        return outputs
    }

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

        // White ground first: JPEG has no alpha channel, so transparent page
        // areas must land on white rather than black.
        context.setFillColor(CGColor(gray: 1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: pixelWidth, height: pixelHeight))

        context.scaleBy(x: scale, y: scale)
        PDFPageReplay.replay(page: page, into: context)

        guard let image = context.makeImage() else {
            throw ToolboxError.encodeFailed("PDF page")
        }
        return image
    }

    private static func encode(
        _ image: CGImage,
        to output: URL,
        format: ImageFormat,
        quality: Double
    ) throws {
        guard let destination = CGImageDestinationCreateWithURL(
            output as CFURL, format.utType.identifier as CFString, 1, nil
        ) else {
            throw ToolboxError.writeFailed(output)
        }
        var properties: [CFString: Any] = [:]
        if format == .jpeg {
            properties[kCGImageDestinationLossyCompressionQuality] = quality
        }
        CGImageDestinationAddImage(destination, image, properties as CFDictionary)
        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.writeFailed(output)
        }
    }
}
