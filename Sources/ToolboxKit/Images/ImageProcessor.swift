import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// One image transform: decode → optional resize → encode.
///
/// Convert, compress and resize are all the same operation with different
/// options, so they share this single implementation.
public struct ImageProcessor: Sendable {

    public struct Options: Sendable {
        /// Output format. `nil` keeps the input's format.
        public var format: ImageFormat?
        /// 0.0…1.0, ignored by lossless formats.
        public var quality: Double
        public var resize: ResizeSpec
        /// Drop EXIF/GPS/maker notes. Location data lives here, so it defaults on
        /// for lossy re-encodes where the user is already accepting a rewrite.
        public var stripMetadata: Bool
        public var allowUpscale: Bool
        /// Keep the original if the "optimized" file came out bigger.
        public var keepSmallerOriginal: Bool
        public var suffix: String
        public var location: OutputLocation

        public init(
            format: ImageFormat? = nil,
            quality: Double = 0.8,
            resize: ResizeSpec = .none,
            stripMetadata: Bool = false,
            allowUpscale: Bool = false,
            keepSmallerOriginal: Bool = false,
            suffix: String = "",
            location: OutputLocation = .alongsideInput
        ) {
            self.format = format
            self.quality = quality
            self.resize = resize
            self.stripMetadata = stripMetadata
            self.allowUpscale = allowUpscale
            self.keepSmallerOriginal = keepSmallerOriginal
            self.suffix = suffix
            self.location = location
        }
    }

    public struct Result: Sendable {
        public let input: URL
        public let output: URL
        public let originalBytes: Int64
        public let newBytes: Int64
        public let pixelSize: CGSize
        public let originalPixelSize: CGSize
        /// True when we fell back to copying the original because the re-encode
        /// was larger.
        public let keptOriginal: Bool

        /// Negative means the file grew.
        public var savedFraction: Double {
            guard originalBytes > 0 else { return 0 }
            return Double(originalBytes - newBytes) / Double(originalBytes)
        }
    }

    public init() {}

    public func run(_ input: URL, options: Options) throws -> Result {
        guard ImageFormat.isReadable(input) else {
            throw ToolboxError.unsupportedInput(input.pathExtension)
        }

        // kCGImageSourceShouldCache false: we touch each pixel buffer once, so
        // caching only inflates memory during batch runs.
        let sourceOptions: [CFString: Any] = [kCGImageSourceShouldCache: false]
        guard let source = CGImageSourceCreateWithURL(input as CFURL, sourceOptions as CFDictionary),
              CGImageSourceGetCount(source) > 0
        else {
            throw ToolboxError.decodeFailed(input)
        }

        let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]

        // Prefer the declared pixel dimensions; they're cheap and correct even
        // before a full decode.
        let declaredWidth = (properties?[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue
        let declaredHeight = (properties?[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue

        let outputFormat = options.format ?? Self.inferredFormat(from: input, source: source)
        guard outputFormat.canEncode else {
            throw ToolboxError.unsupportedOutput(outputFormat.displayName)
        }

        var originalSize = CGSize(width: declaredWidth ?? 0, height: declaredHeight ?? 0)
        let target = options.resize.target(for: originalSize, allowUpscale: options.allowUpscale)

        // An animated GIF or a multi-page file must not come back as frame 0
        // reported as a success, so every frame goes through the same transform
        // — or the run fails and says which frames would have been lost.
        let sequence = try ImageFrameSequence.read(
            from: source,
            input: input,
            requestedFormat: options.format,
            resize: options.resize,
            target: target
        )

        let image: CGImage
        if let sequence {
            image = sequence.firstImage
        } else if let target {
            // Thumbnail path does the decode and scale in one step and honours
            // the EXIF orientation, which is what makes rotated iPhone photos
            // come out upright.
            let maxSide = Int(max(target.width, target.height).rounded())
            let thumbOptions: [CFString: Any] = [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: maxSide,
                kCGImageSourceShouldCacheImmediately: true,
            ]
            guard let scaled = CGImageSourceCreateThumbnailAtIndex(
                source, 0, thumbOptions as CFDictionary
            ) else {
                throw ToolboxError.decodeFailed(input)
            }

            if case .exact = options.resize {
                // Thumbnailing preserves the ratio, so an exact request needs a
                // real redraw into the requested box.
                image = try Self.redraw(scaled, to: target)
            } else {
                image = scaled
            }
        } else {
            guard let full = CGImageSourceCreateImageAtIndex(source, 0, sourceOptions as CFDictionary) else {
                throw ToolboxError.decodeFailed(input)
            }
            image = full
        }

        if originalSize.width == 0 {
            originalSize = CGSize(width: image.width, height: image.height)
        }

        let output = OutputNaming.destination(
            for: input,
            in: options.location,
            suffix: options.suffix,
            extension: sequence?.fileExtension ?? outputFormat.fileExtension
        )

        if let sequence {
            try sequence.write(to: output, quality: options.quality)
        } else {
            try Self.write(
                image: image,
                to: output,
                format: outputFormat,
                quality: options.quality,
                sourceProperties: options.stripMetadata ? nil : properties
            )
        }

        let originalBytes = OutputNaming.fileSize(of: input)
        let newBytes = OutputNaming.fileSize(of: output)

        // Re-encoding can easily produce a *bigger* file — HEIC is roughly twice
        // as efficient as JPEG, and PNG is far larger than either for photos. A
        // tool whose job is "make this smaller" must never hand back something
        // larger, so fall back to the original bytes.
        //
        // Skipped when resizing, where a size change is the point and comparing
        // against the original is meaningless.
        if options.keepSmallerOriginal, originalBytes > 0, newBytes >= originalBytes, target == nil {
            try? FileManager.default.removeItem(at: output)

            // Copy under the *input's* extension: writing HEIC bytes to a .png
            // path would produce a file whose name lies about its contents.
            let fallback = OutputNaming.destination(
                for: input,
                in: options.location,
                suffix: options.suffix,
                extension: input.pathExtension.lowercased()
            )
            try FileManager.default.copyItem(at: input, to: fallback)

            return Result(
                input: input,
                output: fallback,
                originalBytes: originalBytes,
                newBytes: originalBytes,
                pixelSize: originalSize,
                originalPixelSize: originalSize,
                keptOriginal: true
            )
        }

        return Result(
            input: input,
            output: output,
            originalBytes: originalBytes,
            newBytes: newBytes,
            pixelSize: CGSize(width: image.width, height: image.height),
            originalPixelSize: originalSize,
            keptOriginal: false
        )
    }

    // MARK: - Encoding

    private static func write(
        image: CGImage,
        to output: URL,
        format: ImageFormat,
        quality: Double,
        sourceProperties: [CFString: Any]?
    ) throws {
        guard let destination = CGImageDestinationCreateWithURL(
            output as CFURL, format.utType.identifier as CFString, 1, nil
        ) else {
            throw ToolboxError.writeFailed(output)
        }

        var properties: [CFString: Any] = [:]

        if format.supportsQuality {
            properties[kCGImageDestinationLossyCompressionQuality] =
                min(max(quality, 0.0), 1.0)
        } else {
            // PNG/TIFF: ask ImageIO for its best effort rather than its fastest.
            properties[kCGImageDestinationOptimizeColorForSharing] = true
        }

        if let sourceProperties {
            // Carry over the tags worth keeping (orientation is already baked
            // into the pixels, so it must not be copied — that would rotate the
            // image a second time).
            for key in [
                kCGImagePropertyExifDictionary,
                kCGImagePropertyTIFFDictionary,
                kCGImagePropertyIPTCDictionary,
                kCGImagePropertyGPSDictionary,
                kCGImagePropertyDPIWidth,
                kCGImagePropertyDPIHeight,
            ] where sourceProperties[key] != nil {
                properties[key] = sourceProperties[key]
            }
        }

        CGImageDestinationAddImage(destination, image, properties as CFDictionary)

        guard CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.encodeFailed(format.displayName)
        }
    }

    /// Draws into an exact box, ignoring the source aspect ratio.
    /// Not private so each frame of an animation scales identically.
    static func redraw(_ image: CGImage, to size: CGSize) throws -> CGImage {
        let width = Int(size.width.rounded())
        let height = Int(size.height.rounded())

        guard let context = CGContext(
            data: nil,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB) ?? image.colorSpace
                ?? CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else {
            throw ToolboxError.encodeFailed("bitmap")
        }

        context.interpolationQuality = .high
        context.draw(image, in: CGRect(origin: .zero, size: size))

        guard let result = context.makeImage() else {
            throw ToolboxError.encodeFailed("bitmap")
        }
        return result
    }

    private static func inferredFormat(from url: URL, source: CGImageSource) -> ImageFormat {
        if let type = CGImageSourceGetType(source) as String?,
           let match = ImageFormat.allCases.first(where: { $0.utType.identifier == type }) {
            return match
        }
        let ext = url.pathExtension.lowercased()
        return ImageFormat.allCases.first { $0.fileExtension == ext } ?? .png
    }
}
