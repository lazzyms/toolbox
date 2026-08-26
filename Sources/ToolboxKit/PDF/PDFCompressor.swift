import Foundation
import CoreGraphics
import ImageIO
import PDFKit
import UniformTypeIdentifiers

public struct PDFCompressOptions: Sendable {
    public var dpi: Int
    /// JPEG quality, 0–1.
    public var quality: Double

    public init(dpi: Int = 150, quality: Double = 0.75) {
        self.dpi = dpi
        self.quality = quality
    }
}

/// Shrinks a PDF by rasterising every page at a chosen DPI and re-embedding
/// the result as JPEG — the honest offline equivalent of what web compressors
/// do. PDFKit cannot recompress existing image XObjects, so this is the one
/// route that reliably shrinks scan-heavy documents; the cost is that text
/// becomes pixels, which the UI discloses plainly.
public enum PDFCompressor {
    public struct Result: Sendable {
        public let output: URL
        public let pageCount: Int
        public let originalBytes: Int64
        public let newBytes: Int64
    }

    public static func compress(
        _ input: URL,
        options: PDFCompressOptions,
        to location: OutputLocation
    ) throws -> Result {
        if PDFUnlocker.isEncrypted(input) {
            throw ToolboxError.passwordProtected(input)
        }

        let doc = try PDFDocumentIO.open(input)
        let pageCount = doc.pageCount
        guard pageCount > 0 else {
            throw ToolboxError.notAPDF(input)
        }
        let originalBytes = OutputNaming.fileSize(of: input)

        // Pre-flight every page before writing anything, so an absurd page at
        // high DPI fails without leaving a half-built output behind. The caps
        // are the exporter's on purpose — same memory ceiling everywhere.
        let scale = CGFloat(options.dpi) / 72
        for index in 0..<pageCount {
            guard let page = doc.page(at: index) else { continue }
            let media = page.bounds(for: .mediaBox)
            let width = Int((media.width * scale).rounded())
            let height = Int((media.height * scale).rounded())
            guard width <= PDFImageExporter.maxDimension,
                  height <= PDFImageExporter.maxDimension,
                  width * height <= PDFImageExporter.maxPixels
            else {
                throw ToolboxError.resolutionTooLarge(width, height)
            }
        }

        let output = OutputNaming.destination(
            for: input, in: location, suffix: "-compressed", extension: "pdf"
        )

        // The constructor's mediaBox only seeds the file; each page below
        // declares its own box, so mixed page sizes survive.
        var seedBox = doc.page(at: 0)?.bounds(for: .mediaBox) ?? CGRect(x: 0, y: 0, width: 612, height: 792)
        guard let context = CGContext(output as CFURL, mediaBox: &seedBox, nil) else {
            throw ToolboxError.writeFailed(output)
        }

        for index in 0..<pageCount {
            guard let page = doc.page(at: index) else { continue }
            let media = page.bounds(for: .mediaBox)
            let pixelWidth = Int((media.width * scale).rounded())
            let pixelHeight = Int((media.height * scale).rounded())
            guard pixelWidth > 0, pixelHeight > 0 else { continue }

            let rendered = try render(page: page, pixelWidth: pixelWidth, pixelHeight: pixelHeight, scale: scale)
            let jpeg = try jpegData(of: rendered, quality: options.quality)

            // Round-trip through real JPEG data so Quartz embeds a DCTDecode
            // stream instead of lossless pixels — that stream is the entire
            // size win. Drawing the raw render would embed it uncompressed
            // and could inflate the file.
            guard let compressed = decoded(jpeg) else {
                throw ToolboxError.encodeFailed("PDF page")
            }

            var box = media
            context.beginPage(mediaBox: &box)
            context.draw(compressed, in: box)
            context.endPage()
        }
        context.closePDF()

        // "Compress" must never inflate: if the rasterised copy isn't strictly
        // smaller, the original is already optimal — remove the copy and say so.
        let newBytes = OutputNaming.fileSize(of: output)
        guard newBytes < originalBytes else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.noGain
        }

        return Result(
            output: output,
            pageCount: pageCount,
            originalBytes: originalBytes,
            newBytes: newBytes
        )
    }

    /// Same white-ground replay as the exporter's render(): JPEG has no alpha,
    /// so transparent page areas must land on white rather than black, and the
    /// replay flip keeps content upright.
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

        context.setFillColor(CGColor(gray: 1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: pixelWidth, height: pixelHeight))

        context.scaleBy(x: scale, y: scale)
        PDFPageReplay.replay(page: page, into: context)

        guard let image = context.makeImage() else {
            throw ToolboxError.encodeFailed("PDF page")
        }
        return image
    }

    private static func jpegData(of image: CGImage, quality: Double) throws -> Data {
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            data, UTType.jpeg.identifier as CFString, 1, nil
        ) else {
            throw ToolboxError.encodeFailed("PDF page")
        }
        CGImageDestinationAddImage(destination, image, [
            kCGImageDestinationLossyCompressionQuality: quality,
        ] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.encodeFailed("PDF page")
        }
        return data as Data
    }

    private static func decoded(_ data: Data) -> CGImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(source, 0, nil)
    }
}
