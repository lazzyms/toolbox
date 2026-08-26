import CoreGraphics
import CoreImage
import CoreImage.CIFilterBuiltins
import Foundation
import ImageIO
import Vision

/// Batch face anonymisation: detect faces with Vision, blur each region with
/// CoreImage, write a fresh file next to the input.
///
/// The privacy promise here is honesty, not magic: detection misses faces
/// (profiles, occlusion, small background figures), so callers get the per-file
/// count back and must surface it. A run that finds nothing refuses to write a
/// "-blurred" copy at all — an unchanged file wearing that suffix would tell
/// the user the photo is safe to share when nothing happened.
///
/// The blur itself is genuinely destructive: the output is a new render whose
/// pixels are neighbourhood averages, no metadata is carried over, and the
/// original on disk is never touched.
public enum ImageFaceBlurrer {

    public struct Options: Sendable, Equatable {
        /// Minimum blur radius in pixels. Faces much larger than a few hundred
        /// pixels are blurred harder (see `effectiveRadius`) so a foreground
        /// portrait and a small background figure end up equally unreadable.
        public var radius: Double
        /// Lossy outputs only; PNG-class formats ignore it.
        public var quality: Double

        public init(radius: Double = 12, quality: Double = 0.85) {
            self.radius = radius
            self.quality = quality
        }
    }

    /// One file's outcome. `faceCount` exists so the UI can say "found 3
    /// faces" — the number a user needs in order to notice detection came up
    /// short.
    public struct Result: Sendable, Equatable {
        public let output: URL
        public let faceCount: Int

        public init(output: URL, faceCount: Int) {
            self.output = output
            self.faceCount = faceCount
        }
    }

    /// One context for the whole batch: construction is expensive enough that
    /// per-file construction would dominate small batches, `CIContext` is
    /// thread-safe, and each image is touched once so caching intermediates
    /// only spends memory.
    private static let context = CIContext(options: [.cacheIntermediates: false])

    /// `detector` is injectable so tests can pin naming, collision and
    /// frame-safety behaviour without betting on a live model agreeing with
    /// them; production callers take the Vision default.
    public static func run(
        _ input: URL,
        options: Options = Options(),
        location: OutputLocation = .alongsideInput,
        detector: @Sendable (CGImage) throws -> [CGRect] = ImageFaceBlurrer.detectFaces
    ) throws -> Result {
        guard ImageFormat.isReadable(input) else {
            throw ToolboxError.unsupportedInput(input.pathExtension)
        }

        // kCGImageSourceShouldCache false: pixels are touched once per file,
        // so a decode cache only inflates memory across a large batch.
        let sourceOptions: [CFString: Any] = [kCGImageSourceShouldCache: false]
        guard let source = CGImageSourceCreateWithURL(input as CFURL, sourceOptions as CFDictionary),
              CGImageSourceGetCount(source) > 0
        else {
            throw ToolboxError.decodeFailed(input)
        }

        let format = outputFormat(for: input, source: source)

        // Blurring rewrites frame 0's pixels, so anything multi-frame would
        // lose its other frames behind a success report — refuse instead.
        let frames = CGImageSourceGetCount(source)
        if frames > 1 {
            throw ToolboxError.wouldDropFrames(input, frames: frames, format: format.displayName)
        }

        guard let image = uprightImage(at: input) else {
            throw ToolboxError.decodeFailed(input)
        }

        let faces = try detector(image)
        guard !faces.isEmpty else {
            throw ToolboxError.noFacesDetected(input)
        }

        let rendered = try blurring(faces: faces, in: image, options: options)

        let output = OutputNaming.destination(
            for: input,
            in: location,
            suffix: "-blurred",
            extension: format.fileExtension
        )
        try encode(rendered, to: output, format: format, quality: options.quality)
        return Result(output: output, faceCount: faces.count)
    }

    // MARK: - Pixel maths (the testable core)

    /// Blurs `rects` out of `image` at one shared `radius` (pixels of Gaussian
    /// sigma-ish strength) and returns a new bitmap; the input never changes.
    ///
    /// Rects use image pixel coordinates with a top-left origin — the display
    /// convention fixtures and Vision conversions both speak after mapping.
    /// Everything outside the rects is the untouched source render composited
    /// back through a mask, so unblurred regions carry no resampling cost.
    ///
    /// This function holds the tool's entire pixel maths and depends on
    /// neither Vision nor files, so tests can pin placement and contrast
    /// collapse directly instead of trusting a detector to cooperate.
    public static func blur(rects: [CGRect], in image: CGImage, radius: Double) throws -> CGImage {
        guard !rects.isEmpty else { return image }
        let clampedRadius = min(max(radius, 0.5), 200)
        let bounds = CGRect(x: 0, y: 0, width: image.width, height: image.height)

        // CoreImage's origin is the bottom-left corner; flip once here so the
        // rest of the pipeline stays in a single coordinate system. Getting
        // this backwards mirrors every rect — exactly the bug #34 warns about.
        let regions = rects.map { flip($0, height: bounds.height) }

        // Edge pixels extend outward first: CIGaussianBlur samples past the
        // image border, and without the clamp a rect touching an edge fades
        // toward transparent black instead of staying opaque.
        let clamp = CIFilter.affineClamp()
        clamp.inputImage = CIImage(cgImage: image)
        clamp.transform = CGAffineTransform.identity

        let gaussian = CIFilter.gaussianBlur()
        gaussian.inputImage = clamp.outputImage
        gaussian.radius = Float(clampedRadius)

        let blend = CIFilter.blendWithMask()
        blend.inputImage = gaussian.outputImage
        blend.backgroundImage = CIImage(cgImage: image)
        blend.maskImage = mask(withRegions: regions, size: bounds.size)

        guard let merged = blend.outputImage?.cropped(to: bounds) else {
            throw ToolboxError.encodeFailed("blur")
        }

        // Keep the source's own RGB space so a Display P3 photo isn't quietly
        // flattened to sRGB — same rule as the rest of the toolkit.
        let space: CGColorSpace
        if let sourceSpace = image.colorSpace, sourceSpace.model == .rgb {
            space = sourceSpace
        } else {
            space = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
        }

        guard let rendered = context.createCGImage(
            merged, from: bounds, format: .RGBA8, colorSpace: space
        ) else {
            throw ToolboxError.encodeFailed("blur")
        }
        return rendered
    }

    /// The blur radius a face actually receives: the slider value acts as a
    /// floor, scaled up for large faces so anonymisation depth tracks size.
    ///
    /// A purely absolute radius treats a 60-pixel background figure and a
    /// 600-pixel portrait identically, leaving the portrait recognisable.
    /// A quarter of the face's short side destroys detail at any scale while
    /// the slider keeps veto power for users who want more.
    public static func effectiveRadius(sliderRadius: Double, faceSize: CGSize) -> Double {
        let shortSide = min(abs(faceSize.width), abs(faceSize.height))
        return max(sliderRadius, shortSide / 4)
    }

    /// Padding added around a detected box before blurring. Detection boxes
    /// hug the visible skin; a tight blur often leaves hairline, chin or ears
    /// identifiable. Twenty percent each side covers those without swallowing
    /// bystanders.
    public static func expanded(_ rect: CGRect, fraction: Double = 0.2) -> CGRect {
        guard !rect.isEmpty else { return rect }
        let dx = rect.width * CGFloat(fraction)
        let dy = rect.height * CGFloat(fraction)
        return rect.insetBy(dx: -dx, dy: -dy)
    }

    // MARK: - Detection

    /// Runs Vision's face rectangles request over an already-upright bitmap
    /// and returns boxes in image pixel coordinates with a top-left origin —
    /// the same convention `blur(rects:in:radius:)` speaks.
    ///
    /// Detection quality is weather: profiles, heavy occlusion and small
    /// background faces go missing, and model revisions shift results between
    /// OS releases. Treat the returned count as "found", never "all".
    public static func detectFaces(in image: CGImage) throws -> [CGRect] {
        let request = VNDetectFaceRectanglesRequest()
        let handler = VNImageRequestHandler(cgImage: image)
        do {
            try handler.perform([request])
        } catch {
            throw ToolboxError.faceDetectionFailed(error.localizedDescription)
        }

        let width = CGFloat(image.width)
        let height = CGFloat(image.height)
        var rects: [CGRect] = []
        for observation in request.results ?? [] {
            let box = observation.boundingBox
            // Vision normalises to 0…1 with a bottom-left origin; flipping y
            // here is what keeps faces from blurring their mirror images.
            let rect = CGRect(
                x: box.origin.x * width,
                y: (1 - box.minY - box.height) * height,
                width: box.width * width,
                height: box.height * height
            )
            if rect.width >= 1, rect.height >= 1 { rects.append(rect) }
        }
        return rects
    }

    // MARK: - Pipeline internals

    /// Applies one detection result: expands each box past the detected skin,
    /// groups faces by effective radius so similar sizes share one render,
    /// and blurs each group in a single masked pass.
    private static func blurring(
        faces: [CGRect], in image: CGImage, options: Options
    ) throws -> CGImage {
        var groups: [Int: [CGRect]] = [:]
        for face in faces {
            let padded = expanded(face)
            let radius = effectiveRadius(sliderRadius: options.radius, faceSize: face.size)
            groups[Int(radius.rounded()), default: []].append(padded)
        }

        var result = image
        for radius in groups.keys.sorted() {
            result = try blur(rects: groups[radius] ?? [], in: result, radius: Double(radius))
        }
        return result
    }

    private static func flip(_ rect: CGRect, height: CGFloat) -> CGRect {
        CGRect(x: rect.minX, y: height - rect.maxY, width: rect.width, height: rect.height)
    }

    /// A black canvas with white where the rects land, handed to
    /// CIBlendWithMask: white takes the blurred input, black keeps the
    /// original. Built with CGContext because its fill coordinates agree with
    /// CI's bottom-left origin once the rects have been flipped.
    private static func mask(withRegions regions: [CGRect], size: CGSize) -> CIImage {
        guard let context = CGContext(
            data: nil,
            width: max(1, Int(size.width)),
            height: max(1, Int(size.height)),
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else {
            // Unreachable in practice; a black mask blurs nothing, which is
            // the safe direction for a privacy tool to fail.
            return CIImage(color: .black).cropped(to: CGRect(origin: .zero, size: size))
        }
        context.setFillColor(CGColor(gray: 0, alpha: 1))
        context.fill(CGRect(origin: .zero, size: size))
        context.setFillColor(CGColor(gray: 1, alpha: 1))
        for region in regions { context.fill(region) }
        return CIImage(cgImage: context.makeImage()!)
    }

    /// Decodes with any EXIF orientation already applied — detection and
    /// blurring must see the photo the user sees, not the raw sensor buffer.
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

    /// The input's own format when this Mac can write it, else PNG — same
    /// rule as the tone adjuster.
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

        // No metadata is carried over: this render must leak less than the
        // source, and copying an orientation tag would rotate already-upright
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
