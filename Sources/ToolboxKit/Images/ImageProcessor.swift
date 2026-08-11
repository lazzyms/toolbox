import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// One image transform: decode → operations → encode.
///
/// Convert, compress and resize are all the same operation with different
/// options, so they share this single implementation. The middle is an ordered
/// `[ImageOperation]`, so a new image tool is a new operation rather than a new
/// decode/encode path with its own chances to get orientation, the
/// no-inflation guard or output naming wrong.
public struct ImageProcessor: Sendable {

    public struct Options: Sendable {
        /// Output format. `nil` keeps the input's format.
        public var format: ImageFormat?
        /// 0.0…1.0, ignored by lossless formats.
        public var quality: Double
        /// The middle of the pipeline, applied in the order given. Order is
        /// significant: cropping then resizing keeps a different region than
        /// resizing then cropping.
        public var operations: [ImageOperation]
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
            operations: [ImageOperation] = [],
            stripMetadata: Bool = false,
            allowUpscale: Bool = false,
            keepSmallerOriginal: Bool = false,
            suffix: String = "",
            location: OutputLocation = .alongsideInput
        ) {
            self.format = format
            self.quality = quality
            self.operations = operations
            self.stripMetadata = stripMetadata
            self.allowUpscale = allowUpscale
            self.keepSmallerOriginal = keepSmallerOriginal
            self.suffix = suffix
            self.location = location
        }

        /// Resizing on its own, which is what every caller wanted before the
        /// pipeline existed. `resize` deliberately has no default value: that
        /// is what keeps this unambiguous against the initialiser above.
        public init(
            format: ImageFormat? = nil,
            quality: Double = 0.8,
            resize: ResizeSpec,
            stripMetadata: Bool = false,
            allowUpscale: Bool = false,
            keepSmallerOriginal: Bool = false,
            suffix: String = "",
            location: OutputLocation = .alongsideInput
        ) {
            self.init(
                format: format,
                quality: quality,
                operations: [.resize(resize)],
                stripMetadata: stripMetadata,
                allowUpscale: allowUpscale,
                keepSmallerOriginal: keepSmallerOriginal,
                suffix: suffix,
                location: location
            )
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

        // Pick the route through the pipeline before decoding anything: the
        // cheap one for a lone resize can only be taken during the decode.
        let transform = ImageTransform(
            operations: options.operations,
            sourceSize: originalSize,
            orientation: Self.orientation(from: properties),
            allowUpscale: options.allowUpscale
        )

        // An animated GIF or a multi-page file must not come back as frame 0
        // reported as a success, so every frame goes through the same transform
        // — or the run fails and says which frames would have been lost.
        let sequence = try ImageFrameSequence.read(
            from: source,
            input: input,
            requestedFormat: options.format,
            transform: transform
        )

        let rendered: RenderedImage
        if let sequence {
            rendered = RenderedImage(
                image: sequence.firstImage,
                // Frames are written from pixels with no metadata copied at all.
                orientationBaked: false,
                didTransform: sequence.didTransform
            )
        } else {
            rendered = try transform.decode(from: source, at: 0, input: input)
        }
        let image = rendered.image

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
                sourceProperties: options.stripMetadata ? nil : properties,
                dropOrientation: rendered.orientationBaked
            )
        }

        let originalBytes = OutputNaming.fileSize(of: input)
        let newBytes = OutputNaming.fileSize(of: output)

        // Re-encoding can easily produce a *bigger* file — HEIC is roughly twice
        // as efficient as JPEG, and PNG is far larger than either for photos. A
        // tool whose job is "make this smaller" must never hand back something
        // larger, so fall back to the original bytes.
        //
        // Skipped whenever an operation actually changed the pixels: a resize,
        // crop, rotation or flip is a change the user asked for, so handing the
        // original back instead would silently discard it. Operations that
        // resolved to nothing for this image don't count, which is what keeps a
        // plain convert or compress under the guard.
        if options.keepSmallerOriginal, originalBytes > 0, newBytes >= originalBytes,
           !rendered.didTransform {
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
        sourceProperties: [CFString: Any]?,
        dropOrientation: Bool
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
            // That flag also converts the colours to sRGB "for sharing", which
            // would flatten a wide-gamut (Display P3) photo and dull the
            // colours — exactly what the pipeline must not do. It is a no-op
            // for pixels that already are sRGB, so only those get it.
            if Self.isAlreadySRGB(image.colorSpace) {
                properties[kCGImageDestinationOptimizeColorForSharing] = true
            }
        }

        if let sourceProperties {
            // Carry over the tags worth keeping. The top-level orientation is
            // never among them.
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

            // When the decode turned the pixels upright, the orientation has
            // already been spent and writing the tag would rotate the image a
            // second time on display. Omitting the top-level key is not enough:
            // ImageIO reports the same value inside the TIFF dictionary, which
            // is copied wholesale above.
            if dropOrientation, var tiff = properties[kCGImagePropertyTIFFDictionary]
                as? [CFString: Any] {
                tiff[kCGImagePropertyTIFFOrientation] = nil
                properties[kCGImagePropertyTIFFDictionary] = tiff
            }
        }

        CGImageDestinationAddImage(destination, image, properties.isEmpty ? nil : properties as CFDictionary)

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

    /// The orientation the file records, or `.up` when it records none.
    private static func orientation(
        from properties: [CFString: Any]?
    ) -> CGImagePropertyOrientation {
        guard let raw = (properties?[kCGImagePropertyOrientation] as? NSNumber)?.uint32Value,
              let orientation = CGImagePropertyOrientation(rawValue: raw)
        else { return .up }
        return orientation
    }

    /// Whether `kCGImageDestinationOptimizeColorForSharing` is a no-op, so the
    /// flag can be applied without flattening a wide gamut: the image already
    /// is sRGB, or carries no profile worth keeping.
    private static func isAlreadySRGB(_ space: CGColorSpace?) -> Bool {
        guard let space else { return true }
        // Greyscale and CMYK pixels can't describe an RGBA buffer and ImageIO
        // picks the conversion itself; leave today's behaviour alone there.
        guard space.model == .rgb else { return true }
        if let name = space.name {
            // Named sRGB is canonical. A different *name* (Display P3, Adobe
            // RGB…) means a wider gamut that sharing would destroy.
            return name == CGColorSpace.sRGB
        }
        // A nameless RGB space is ICC-based. ImageIO decodes ordinary sRGB
        // files to the named space above, so one of these carries a profile
        // that must not be thrown away.
        return false
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
