import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// The two multi-page TIFF tools: a scanner-style multi-page file split back
/// into one image per page, and a batch of stills bound into one such file.
///
/// Both are thin wrappers over ImageIO's own container support — pages have no
/// timing to preserve (unlike GIF frames), so the plain index-based read and
/// counted-destination write are all that's needed.
public enum TIFFTools {

    /// The compression written into the output TIFF. Some consumers of
    /// multi-page TIFF (document management, legal) only accept specific ones.
    public enum Compression: Int, Sendable, CaseIterable {
        case none = 1
        case lzw = 5
        case packbits = 32773

        // The `kCGImagePropertyTIFFCompression…` constants are C macros that
        // never import into Swift, so the standard TIFF tag values stand in.
        var propertyValue: Int {
            switch self {
            case .none: return 1
            case .lzw: return 5
            case .packbits: return 32773
            }
        }

        public var displayName: String {
            switch self {
            case .none: return "None"
            case .lzw: return "LZW"
            case .packbits: return "PackBits"
            }
        }
    }

    // MARK: Split

    /// Every page becomes its own image file named `<stem>-frame-N`.
    ///
    /// A single-page input is legitimate here — it yields exactly one output,
    /// which is how a stray single-page TIFF can be converted on the way out.
    /// Pages come back as stored: multi-page TIFF carries no orientation or
    /// timing metadata worth second-guessing.
    public static func split(
        _ input: URL,
        format: ImageFormat = .png,
        to location: OutputLocation = .alongsideInput
    ) throws -> [URL] {
        let sourceOptions: [CFString: Any] = [kCGImageSourceShouldCache: false]
        guard let source = CGImageSourceCreateWithURL(
            input as CFURL, sourceOptions as CFDictionary
        ) else {
            throw ToolboxError.decodeFailed(input)
        }
        let pageCount = CGImageSourceGetCount(source)
        guard pageCount > 0 else {
            throw ToolboxError.decodeFailed(input)
        }

        var outputs: [URL] = []
        outputs.reserveCapacity(pageCount)
        for index in 0..<pageCount {
            guard let page = CGImageSourceCreateImageAtIndex(source, index, [
                kCGImageSourceShouldCacheImmediately: true,
            ] as CFDictionary) else {
                throw ToolboxError.decodeFailed(input)
            }
            let output = OutputNaming.destination(
                for: input,
                in: location,
                suffix: "-frame-\(index + 1)",
                extension: format.fileExtension
            )
            try write(page, to: output, format: format)
            outputs.append(output)
        }
        return outputs
    }

    // MARK: Combine

    /// Several images become one multi-page TIFF. Queue order is page order.
    ///
    /// A single image is accepted and produces a legitimate one-page TIFF.
    public static func combine(
        _ inputs: [URL],
        to location: OutputLocation = .alongsideInput,
        compression: Compression = .lzw
    ) throws -> URL {
        guard !inputs.isEmpty else {
            throw ToolboxError.emptySelection
        }

        for input in inputs where !ImageFormat.isReadable(input) {
            throw ToolboxError.unsupportedInput(input.pathExtension)
        }

        // Decode everything before the first byte of output is written, so a
        // bad file anywhere in the queue leaves nothing behind at all.
        let pages = try inputs.map { try decodedPage($0) }

        let output = OutputNaming.destination(
            for: inputs[0],
            in: location,
            suffix: "-combined",
            extension: "tiff"
        )

        guard let destination = CGImageDestinationCreateWithURL(
            output as CFURL, UTType.tiff.identifier as CFString, pages.count, nil
        ) else {
            throw ToolboxError.writeFailed(output)
        }

        let properties: [CFString: Any] = [
            kCGImagePropertyTIFFCompression: compression.propertyValue,
        ]
        for page in pages {
            CGImageDestinationAddImage(destination, page, properties as CFDictionary)
        }
        guard CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.encodeFailed("TIFF")
        }

        return output
    }

    // MARK: - Helpers

    /// Decodes one image upright — the thumbnail route applies the EXIF
    /// orientation during the decode, so a phone photo lands in the TIFF the
    /// way it is displayed rather than lying on its side.
    private static func decodedPage(_ input: URL) throws -> CGImage {
        let sourceOptions: [CFString: Any] = [kCGImageSourceShouldCache: false]
        guard let source = CGImageSourceCreateWithURL(
            input as CFURL, sourceOptions as CFDictionary
        ), CGImageSourceGetCount(source) > 0 else {
            throw ToolboxError.decodeFailed(input)
        }

        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            // A cap above any source's longest side never binds; there is no
            // size dial for this tool, so every frame keeps its source size.
            kCGImageSourceThumbnailMaxPixelSize: 100_000,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
        else {
            throw ToolboxError.decodeFailed(input)
        }
        return image
    }

    /// Writes one image as a single-image file of the given format.
    private static func write(_ image: CGImage, to output: URL, format: ImageFormat) throws {
        guard let destination = CGImageDestinationCreateWithURL(
            output as CFURL, format.utType.identifier as CFString, 1, nil
        ) else {
            throw ToolboxError.writeFailed(output)
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.encodeFailed(format.displayName)
        }
    }
}
