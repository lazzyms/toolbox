import CoreGraphics
import CoreImage
import CoreImage.CIFilterBuiltins
import CoreVideo
import Foundation
import ImageIO
import Vision

/// Cuts the subject out of a photo into a transparent PNG.
///
/// Segmentation runs on-device through `VNGenerateForegroundInstanceMaskRequest`
/// (Vision, macOS 14+) — the same engine behind "lift subject from background"
/// in Preview and Photos. Vision is a system framework, so ToolboxKit's
/// dependency-free rule holds.
///
/// Every detected instance is unioned into one mask, and the mask is applied as
/// an alpha channel with its soft edges intact — thresholding to hard alpha is
/// what makes hair and fur look scissor-cut. Output is forced to PNG: any
/// alpha-less format would silently fill the transparency with black.
public enum ImageBackgroundRemover {

    /// One context for the whole batch, for the same reasons as
    /// `ImageToneAdjuster`: construction dominates small batches, the instance
    /// is thread-safe, and nothing here benefits from caching intermediates.
    private static let context = CIContext(options: [.cacheIntermediates: false])

    public static func run(
        _ input: URL,
        location: OutputLocation = .alongsideInput
    ) throws -> URL {
        guard ImageFormat.isReadable(input) else {
            throw ToolboxError.unsupportedInput(input.pathExtension)
        }

        // kCGImageSourceShouldCache false, as in `ImageToneAdjuster`: pixels are
        // touched once per file and segmentation is already memory-hungry.
        let sourceOptions: [CFString: Any] = [kCGImageSourceShouldCache: false]
        guard let source = CGImageSourceCreateWithURL(input as CFURL, sourceOptions as CFDictionary),
              CGImageSourceGetCount(source) > 0
        else {
            throw ToolboxError.decodeFailed(input)
        }

        // A cutout is frame 0 only, so anything multi-frame would lose its
        // other frames behind a success report — refuse like the tone tool does.
        let frames = CGImageSourceGetCount(source)
        if frames > 1 {
            throw ToolboxError.wouldDropFrames(
                input, frames: frames, format: inputFormat(for: input, source: source).displayName
            )
        }

        guard let image = uprightImage(at: input) else {
            throw ToolboxError.decodeFailed(input)
        }

        let mask = try subjectMask(for: image, input: input)
        return try writeCutout(image: compose(image: image, mask: mask), for: input, in: location)
    }

    /// Applies an alpha mask over `image`, keeping soft edges.
    ///
    /// This is the deterministic half of the pipeline — the part tests pin
    /// without trusting a neural network. `mask` must cover the image (the
    /// scaled masks Vision returns do); white keeps a pixel, black clears it,
    /// greys become partial transparency. The result is RGBA8 so it can carry
    /// an alpha channel into the PNG encoder.
    public static func compose(image: CGImage, mask: CGImage) throws -> CGImage {
        var filter = CIFilter.blendWithMask()
        let foreground = CIImage(cgImage: image)
        filter.inputImage = foreground
        // Blend against fully transparent rather than black, or "background"
        // would be a colour swap instead of a cutout.
        filter.backgroundImage = CIImage(color: .clear).cropped(to: foreground.extent)
        filter.maskImage = CIImage(cgImage: mask)

        guard let blended = filter.outputImage else {
            throw ToolboxError.encodeFailed("cutout")
        }
        return try render(blended, preservingSpaceOf: image)
    }

    /// Encodes a finished cutout beside the input (or in the chosen folder),
    /// always as PNG — `-cutout.png`, then `-cutout-1.png` on collision.
    /// Internal because tests drive it directly with hand-built masks.
    static func writeCutout(
        image: CGImage, for input: URL, in location: OutputLocation
    ) throws -> URL {
        let output = OutputNaming.destination(
            for: input, in: location, suffix: "-cutout", extension: ImageFormat.png.fileExtension
        )
        guard let destination = CGImageDestinationCreateWithURL(
            output as CFURL, ImageFormat.png.utType.identifier as CFString, 1, nil
        ) else {
            throw ToolboxError.writeFailed(output)
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.encodeFailed(ImageFormat.png.displayName)
        }
        return output
    }

    /// Runs Vision's subject segmentation over the decoded image and hands back
    /// a mask at full resolution.
    ///
    /// No instances means no clear subject — reported honestly rather than
    /// written out as a blank file, since that would look exactly like success.
    private static func subjectMask(for image: CGImage, input: URL) throws -> CGImage {
        let request = VNGenerateForegroundInstanceMaskRequest()
        let handler = VNImageRequestHandler(cgImage: image)

        // `perform` blocks until recognition finishes, so this stays
        // synchronous despite Vision's asynchronous-looking API.
        do {
            try handler.perform([request])
        } catch {
            throw ToolboxError.segmentationFailed(error.localizedDescription)
        }

        // Union every detected instance: two subjects in frame both belong to
        // the user, not just the biggest one.
        guard let observation = request.results?.first,
              !observation.allInstances.isEmpty
        else {
            throw ToolboxError.noSubjectFound(input)
        }

        let buffer: CVPixelBuffer
        do {
            buffer = try observation.generateScaledMaskForImage(
                forInstances: observation.allInstances, from: handler
            )
        } catch {
            throw ToolboxError.segmentationFailed(error.localizedDescription)
        }
        // The single-channel mask goes through the same render path as any
        // other CIImage so `compose` always sees a plain CGImage.
        return try render(CIImage(cvPixelBuffer: buffer), preservingSpaceOf: nil)
    }

    /// Renders a CIImage to RGBA8. The source's own RGB space is kept when
    /// given one, so a Display P3 photo isn't quietly flattened to sRGB;
    /// non-RGB spaces can't describe an RGBA buffer and fall back to sRGB.
    private static func render(_ image: CIImage, preservingSpaceOf original: CGImage?) throws -> CGImage {
        let space: CGColorSpace
        if let sourceSpace = original?.colorSpace, sourceSpace.model == .rgb {
            space = sourceSpace
        } else {
            space = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
        }
        guard let rendered = context.createCGImage(
            image, from: image.extent, format: .RGBA8, colorSpace: space
        ) else {
            throw ToolboxError.encodeFailed("cutout")
        }
        return rendered
    }

    /// Decodes with any EXIF orientation already applied — the mask comes back
    /// aligned to these upright pixels, and the PNG carries no orientation tag,
    /// so the rotation has to land in the pixels first.
    private static func uprightImage(at url: URL) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        let width = (props?[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue ?? 1
        let height = (props?[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue ?? 1
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: max(width, height),
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }

    /// The input's own format — used only to name it in the dropped-frames
    /// error; this tool always writes PNG regardless.
    private static func inputFormat(for input: URL, source: CGImageSource) -> ImageFormat {
        let detected = (CGImageSourceGetType(source) as String?)
            .flatMap { type in ImageFormat.allCases.first { $0.utType.identifier == type } }
            ?? ImageFormat.allCases.first { $0.fileExtension == input.pathExtension.lowercased() }
        return detected ?? .png
    }
}
