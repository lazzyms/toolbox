import CoreGraphics
import CoreImage
import CoreImage.CIFilterBuiltins
import Foundation
import ImageIO

/// Batch colour and tone adjustments: brightness, contrast, saturation,
/// exposure, temperature/tint, plus grayscale/sepia/invert presets and
/// CoreImage's own auto-enhance.
///
/// A standalone pass rather than an `ImageOperation` because every adjustment
/// here is a whole-image point operation — no geometry, no per-image fit
/// checks — so the tool is one decode → filter chain → encode run. All knobs
/// default to their neutral and a fully neutral run skips the filter chain
/// entirely, so it cannot drift from the source pixels.
public enum ImageToneAdjuster {

    public struct Options: Sendable, Equatable {
        /// −1…1; 0 leaves levels alone.
        public var brightness: Double
        /// 0…2 multiplier; 1 unchanged.
        public var contrast: Double
        /// 0…2 multiplier; 0 flattens to grayscale, 1 unchanged.
        public var saturation: Double
        /// Exposure stops; +1 is one stop brighter.
        public var exposure: Double
        /// Colour temperature in kelvin; 6500 is neutral.
        public var temperature: Double
        /// Green–magenta axis; 0 is neutral.
        public var tint: Double
        public var grayscale: Bool
        public var sepia: Bool
        public var invert: Bool
        /// CoreImage's `autoAdjustmentFilters`, with its crop and red-eye steps
        /// disabled so analysis can never reframe or mark eyes.
        public var autoEnhance: Bool
        /// Lossy outputs only; PNG-class formats ignore it.
        public var quality: Double

        public init(
            brightness: Double = 0,
            contrast: Double = 1,
            saturation: Double = 1,
            exposure: Double = 0,
            temperature: Double = 6500,
            tint: Double = 0,
            grayscale: Bool = false,
            sepia: Bool = false,
            invert: Bool = false,
            autoEnhance: Bool = false,
            quality: Double = 0.85
        ) {
            self.brightness = brightness
            self.contrast = contrast
            self.saturation = saturation
            self.exposure = exposure
            self.temperature = temperature
            self.tint = tint
            self.grayscale = grayscale
            self.sepia = sepia
            self.invert = invert
            self.autoEnhance = autoEnhance
            self.quality = quality
        }

        /// Whether any knob left its neutral — i.e. whether a run would change
        /// pixels at all. Drives both the view's run button and the skip below.
        public var isActive: Bool {
            brightness != 0 || contrast != 1 || saturation != 1 || exposure != 0
                || temperature != 6500 || tint != 0
                || grayscale || sepia || invert || autoEnhance
        }
    }

    /// One context for the whole batch: constructing one is expensive enough
    /// that per-file construction would dominate small batches, `CIContext` is
    /// thread-safe, and each image is touched once so caching intermediates
    /// only spends memory. The linear working space is CoreImage's default,
    /// matching the operation pipeline.
    private static let context = CIContext(options: [.cacheIntermediates: false])

    public static func run(
        _ input: URL,
        options: Options = Options(),
        location: OutputLocation = .alongsideInput
    ) throws -> URL {
        guard ImageFormat.isReadable(input) else {
            throw ToolboxError.unsupportedInput(input.pathExtension)
        }

        // kCGImageSourceShouldCache false: pixels are touched once per file, so
        // a decode cache only inflates memory across a large batch.
        let sourceOptions: [CFString: Any] = [kCGImageSourceShouldCache: false]
        guard let source = CGImageSourceCreateWithURL(input as CFURL, sourceOptions as CFDictionary),
              CGImageSourceGetCount(source) > 0
        else {
            throw ToolboxError.decodeFailed(input)
        }

        let format = outputFormat(for: input, source: source)

        // Adjustments rewrite every pixel of frame 0, so anything multi-frame
        // would lose its other frames behind a success report — refuse instead.
        let frames = CGImageSourceGetCount(source)
        if frames > 1 {
            throw ToolboxError.wouldDropFrames(input, frames: frames, format: format.displayName)
        }

        guard let image = uprightImage(at: input) else {
            throw ToolboxError.decodeFailed(input)
        }

        let rendered = try adjust(image, options: options) ?? image

        let output = OutputNaming.destination(
            for: input,
            in: location,
            suffix: "-adjusted",
            extension: format.fileExtension
        )
        try encode(rendered, to: output, format: format, quality: options.quality)
        return output
    }

    /// Runs the filter chain, or nil when nothing is active — in which case the
    /// decoded pixels are re-encoded as they are.
    private static func adjust(_ image: CGImage, options: Options) throws -> CGImage? {
        guard options.isActive else { return nil }

        var current = CIImage(cgImage: image)

        // Auto-enhance first so its analysis sets up the photo before the
        // user's own values are applied on top of it.
        if options.autoEnhance {
            let analysis: [CIImageAutoAdjustmentOption: Any] = [
                CIImageAutoAdjustmentOption.crop: false,
                CIImageAutoAdjustmentOption.redEye: false,
                CIImageAutoAdjustmentOption.level: false,
                CIImageAutoAdjustmentOption.enhance: true,
            ]
            for filter in current.autoAdjustmentFilters(options: analysis) {
                filter.setValue(current, forKey: kCIInputImageKey)
                if let adjusted = filter.outputImage { current = adjusted }
            }
        }

        if options.exposure != 0 {
            let filter = CIFilter.exposureAdjust()
            filter.inputImage = current
            filter.ev = Float(options.exposure)
            if let adjusted = filter.outputImage { current = adjusted }
        }

        if options.brightness != 0 || options.contrast != 1 || options.saturation != 1 {
            let filter = CIFilter.colorControls()
            filter.inputImage = current
            filter.brightness = Float(options.brightness)
            filter.contrast = Float(options.contrast)
            filter.saturation = Float(options.saturation)
            if let adjusted = filter.outputImage { current = adjusted }
        }

        if options.temperature != 6500 || options.tint != 0 {
            let filter = CIFilter.temperatureAndTint()
            filter.inputImage = current
            filter.neutral = CIVector(x: 6500, y: 0)
            filter.targetNeutral = CIVector(x: CGFloat(options.temperature), y: CGFloat(options.tint))
            if let adjusted = filter.outputImage { current = adjusted }
        }

        if options.grayscale {
            let filter = CIFilter.colorMonochrome()
            filter.inputImage = current
            filter.color = CIColor(red: 1, green: 1, blue: 1)
            filter.intensity = 1
            if let adjusted = filter.outputImage { current = adjusted }
        }

        if options.sepia {
            let filter = CIFilter.sepiaTone()
            filter.inputImage = current
            filter.intensity = 1
            if let adjusted = filter.outputImage { current = adjusted }
        }

        if options.invert {
            let filter = CIFilter.colorInvert()
            filter.inputImage = current
            if let adjusted = filter.outputImage { current = adjusted }
        }

        // Keep the source's own RGB space so a Display P3 photo isn't quietly
        // flattened to sRGB — same rule as the operation pipeline. Non-RGB
        // spaces (greyscale, CMYK) can't describe an RGBA buffer and fall back
        // to sRGB.
        let space: CGColorSpace
        if let sourceSpace = image.colorSpace, sourceSpace.model == .rgb {
            space = sourceSpace
        } else {
            space = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
        }

        guard let rendered = context.createCGImage(
            current, from: current.extent, format: .RGBA8, colorSpace: space
        ) else {
            throw ToolboxError.encodeFailed("bitmap")
        }
        return rendered
    }

    /// Decodes with any EXIF orientation already applied. The output carries no
    /// orientation tag at all (see `encode`), so the rotation must land in the
    /// pixels or the file would display rotated a second time.
    private static func uprightImage(at url: URL) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        let width = (props?[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue ?? 1
        let height = (props?[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue ?? 1
        let thumbnailOptions: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: max(width, height),
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, thumbnailOptions as CFDictionary)
    }

    /// The input's own format when this Mac can write it, else PNG — a DNG
    /// photo still deserves its tone fix, just not as DNG.
    private static func outputFormat(for input: URL, source: CGImageSource) -> ImageFormat {
        let detected = (CGImageSourceGetType(source) as String?)
            .flatMap { type in ImageFormat.allCases.first { $0.utType.identifier == type } }
            ?? ImageFormat.allCases.first { $0.fileExtension == input.pathExtension.lowercased() }
        guard let format = detected, format.canEncode else { return .png }
        return format
    }

    private static func encode(
        _ image: CGImage, to output: URL, format: ImageFormat, quality: Double
    ) throws {
        guard let destination = CGImageDestinationCreateWithURL(
            output as CFURL, format.utType.identifier as CFString, 1, nil
        ) else {
            throw ToolboxError.writeFailed(output)
        }

        var properties: [CFString: Any] = [:]
        if format.supportsQuality {
            properties[kCGImageDestinationLossyCompressionQuality] = min(max(quality, 0), 1)
        }

        // No metadata is carried over. This is a fresh creative render, and
        // copying the source's orientation tag would rotate the already-upright
        // pixels a second time when displayed.
        CGImageDestinationAddImage(
            destination, image, properties.isEmpty ? nil : properties as CFDictionary
        )

        guard CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.encodeFailed(format.displayName)
        }
    }
}
