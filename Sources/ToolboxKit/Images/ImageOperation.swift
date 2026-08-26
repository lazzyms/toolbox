import CoreGraphics
import CoreImage
import CoreImage.CIFilterBuiltins
import Foundation
import ImageIO

/// One step in the middle of the image pipeline.
///
/// `ImageProcessor` is decode → operations → encode, and every image tool is
/// that same pipeline with a different operation list. Keeping the steps in one
/// ordered list rather than one field per tool is what stops each new tool from
/// growing its own decode/encode path, where EXIF orientation, the
/// no-inflation guard and `OutputNaming` would all have to be got right again.
///
/// Order is significant and is applied as given: cropping and then resizing
/// keeps a different part of the image than resizing and then cropping.
public enum ImageOperation: Sendable, Equatable {
    case crop(CropSpec)
    /// Whole quarter turns only, clockwise. Any multiple of 90 is accepted and
    /// normalised; anything else throws rather than silently rounding, because
    /// a free rotation needs a background colour and a new bounding box.
    case rotate(degrees: Int)
    case flip(horizontal: Bool, vertical: Bool)
    case resize(ResizeSpec)

    /// Whether this step can change pixels at all, judged without a source
    /// image. Used to reduce an operation list to the work actually requested,
    /// so a tool that passes `.resize(.none)` still takes the plain
    /// decode-and-re-encode path.
    var mayChangePixels: Bool {
        switch self {
        case .crop(let spec): return spec.mayChangePixels
        // An angle that isn't a right angle counts as a change so the run
        // reaches the renderer and fails there with a reason, rather than
        // being quietly dropped here.
        case .rotate(let degrees): return ImageOperation.normalisedDegrees(degrees) != 0
        case .flip(let horizontal, let vertical): return horizontal || vertical
        case .resize(let spec): return spec.isActive
        }
    }

    static func normalisedDegrees(_ degrees: Int) -> Int {
        ((degrees % 360) + 360) % 360
    }

    /// Quarter turns clockwise, or nil for an angle that isn't a right angle.
    static func quarterTurns(_ degrees: Int) -> Int? {
        let normalised = normalisedDegrees(degrees)
        guard normalised % 90 == 0 else { return nil }
        return normalised / 90
    }
}

// MARK: - Cropping

/// Which region of an image to keep.
///
/// Coordinates are in pixels with the origin at the **top left of the image as
/// displayed**, which is how a user and a UI describe a crop. The renderer
/// converts to CoreImage's bottom-left origin, and applies EXIF orientation
/// first so "top left" means the same thing for a rotated phone photo.
public enum CropSpec: Sendable, Equatable {
    /// An explicit pixel rect, clamped to the image.
    case rect(x: Int, y: Int, width: Int, height: Int)
    /// The largest rect with this aspect ratio that fits, positioned by anchor.
    /// `.aspect(width: 1, height: 1, anchor: .center)` is a centred square.
    case aspect(width: Int, height: Int, anchor: CropAnchor)

    var mayChangePixels: Bool {
        switch self {
        case .rect(_, _, let width, let height): return width > 0 && height > 0
        case .aspect(let width, let height, _): return width > 0 && height > 0
        }
    }

    /// The region to keep, in top-left-origin pixels, or nil when the crop is
    /// impossible or would keep the whole image.
    func rect(for source: CGSize) -> CGRect? {
        guard source.width >= 1, source.height >= 1 else { return nil }
        let bounds = CGRect(origin: .zero, size: source)

        let requested: CGRect
        switch self {
        case .rect(let x, let y, let width, let height):
            guard width > 0, height > 0 else { return nil }
            requested = CGRect(
                x: Double(x), y: Double(y), width: Double(width), height: Double(height)
            )

        case .aspect(let width, let height, let anchor):
            guard width > 0, height > 0 else { return nil }
            // Fit the ratio inside the image, then place it.
            let scale = min(source.width / Double(width), source.height / Double(height))
            let size = CGSize(
                width: max(1, (Double(width) * scale).rounded(.down)),
                height: max(1, (Double(height) * scale).rounded(.down))
            )
            requested = CGRect(origin: anchor.origin(for: size, in: source), size: size)
        }

        let clamped = requested.intersection(bounds)
        guard !clamped.isNull, clamped.width >= 1, clamped.height >= 1 else { return nil }
        // A crop that keeps everything is not a crop, and saying so here keeps
        // it out of the "pixels changed" bookkeeping.
        return clamped == bounds ? nil : clamped
    }
}

/// Where a crop sits when it is smaller than the image.
public enum CropAnchor: String, Sendable, CaseIterable, Identifiable {
    case center, top, bottom, left, right
    case topLeft, topRight, bottomLeft, bottomRight

    public var id: String { rawValue }

    /// 0 is the left/top edge, 1 the right/bottom edge.
    private var unitPoint: CGPoint {
        switch self {
        case .center: return CGPoint(x: 0.5, y: 0.5)
        case .top: return CGPoint(x: 0.5, y: 0)
        case .bottom: return CGPoint(x: 0.5, y: 1)
        case .left: return CGPoint(x: 0, y: 0.5)
        case .right: return CGPoint(x: 1, y: 0.5)
        case .topLeft: return CGPoint(x: 0, y: 0)
        case .topRight: return CGPoint(x: 1, y: 0)
        case .bottomLeft: return CGPoint(x: 0, y: 1)
        case .bottomRight: return CGPoint(x: 1, y: 1)
        }
    }

    /// Top-left origin for a rect of `size` inside `source`.
    func origin(for size: CGSize, in source: CGSize) -> CGPoint {
        let point = unitPoint
        return CGPoint(
            x: max(0, ((source.width - size.width) * point.x).rounded(.down)),
            y: max(0, ((source.height - size.height) * point.y).rounded(.down))
        )
    }
}

// MARK: - Choosing a route through the pipeline

/// How to get from the bytes on disk to the pixels that will be written.
///
/// This is decided once per file, before any decoding, because the cheapest
/// route for the commonest case can't be reached afterwards: a lone resize is
/// better served by ImageIO scaling *during* the decode than by decoding at
/// full size and scaling after.
enum ImageRenderPlan: Equatable {
    /// Nothing to do in the middle — convert and compress.
    case passthrough
    /// Exactly one resize: `CGImageSourceCreateThumbnailAtIndex` decodes and
    /// scales in one step and applies the EXIF orientation while it's at it.
    case thumbnail(ResizeSpec)
    /// Everything else: one CoreImage graph, rendered once.
    case pipeline([ImageOperation])

    init(operations: [ImageOperation]) {
        let active = operations.filter(\.mayChangePixels)
        if active.isEmpty {
            self = .passthrough
        } else if active.count == 1, case .resize(let spec) = active[0] {
            self = .thumbnail(spec)
        } else {
            self = .pipeline(active)
        }
    }
}

/// The pixels a decode produced, plus the two facts the encoder needs to know
/// about how they were produced.
struct RenderedImage {
    let image: CGImage

    /// The decode already turned the pixels upright, so an orientation tag must
    /// not be written alongside them — that would rotate the image a second
    /// time when it is displayed.
    let orientationBaked: Bool

    /// An operation actually changed the pixels, which makes comparing the
    /// output's size against the original's meaningless.
    let didTransform: Bool
}

/// Everything needed to turn one frame of a `CGImageSource` into pixels,
/// resolved once per file so every frame of an animation is treated alike.
struct ImageTransform {
    let plan: ImageRenderPlan
    /// Pre-resolved size for the thumbnail route, which has to know how big to
    /// decode before it decodes. nil on every other route, where each step
    /// resolves against the size it is actually handed.
    let target: CGSize?
    let orientation: CGImagePropertyOrientation
    let allowUpscale: Bool

    init(
        operations: [ImageOperation],
        sourceSize: CGSize,
        orientation: CGImagePropertyOrientation,
        allowUpscale: Bool
    ) {
        let plan = ImageRenderPlan(operations: operations)
        self.plan = plan
        self.orientation = orientation
        self.allowUpscale = allowUpscale
        if case .thumbnail(let spec) = plan {
            target = spec.target(for: sourceSize, allowUpscale: allowUpscale)
        } else {
            target = nil
        }
    }

    /// Decodes one frame and applies the operations to it.
    func decode(from source: CGImageSource, at index: Int, input: URL) throws -> RenderedImage {
        switch plan {
        case .passthrough:
            return RenderedImage(
                image: try Self.fullImage(from: source, at: index, input: input),
                orientationBaked: false,
                didTransform: false
            )

        case .thumbnail(let spec):
            guard let target else {
                // The resize resolved to no change at all — an upscale that
                // wasn't allowed, or a box the image already fits. Decode it
                // exactly as a run with no operations would.
                return RenderedImage(
                    image: try Self.fullImage(from: source, at: index, input: input),
                    orientationBaked: false,
                    didTransform: false
                )
            }

            // One step for decode and scale, and `WithTransform` honours the
            // EXIF orientation, which is what makes rotated phone photos come
            // out upright.
            let maxSide = Int(max(target.width, target.height).rounded())
            let thumbOptions: [CFString: Any] = [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: maxSide,
                kCGImageSourceShouldCacheImmediately: true,
            ]
            guard let scaled = CGImageSourceCreateThumbnailAtIndex(
                source, index, thumbOptions as CFDictionary
            ) else {
                throw ToolboxError.decodeFailed(input)
            }

            var image = scaled
            if case .exact = spec {
                // Thumbnailing preserves the ratio, so an exact request needs a
                // real redraw into the requested box.
                image = try ImageProcessor.redraw(scaled, to: target)
            }
            return RenderedImage(image: image, orientationBaked: true, didTransform: true)

        case .pipeline(let operations):
            return try ImageOperationRenderer.apply(
                operations,
                to: try Self.fullImage(from: source, at: index, input: input),
                orientation: orientation,
                allowUpscale: allowUpscale
            )
        }
    }

    private static func fullImage(
        from source: CGImageSource, at index: Int, input: URL
    ) throws -> CGImage {
        guard let full = CGImageSourceCreateImageAtIndex(
            source, index, [kCGImageSourceShouldCache: false] as CFDictionary
        ) else {
            throw ToolboxError.decodeFailed(input)
        }
        return full
    }
}

// MARK: - Applying operations

/// Applies an operation list to one decoded image.
///
/// Each step is a lazy CoreImage node rather than a bitmap redraw, so a chain
/// of five operations still costs a single render into a single bitmap at the
/// end instead of five full-size intermediates.
enum ImageOperationRenderer {

    /// Shared because building a renderer is expensive enough that a batch
    /// shouldn't pay for it per file, and `CIContext` is safe to share.
    /// `cacheIntermediates` is off because each image is touched once.
    private static let context: CIContext = {
        var options: [CIContextOption: Any] = [.cacheIntermediates: false]
        return CIContext(options: options)
    }()

    static func apply(
        _ operations: [ImageOperation],
        to image: CGImage,
        orientation: CGImagePropertyOrientation,
        allowUpscale: Bool
    ) throws -> RenderedImage {
        var current = CIImage(cgImage: image)
        var changed = false

        // Orientation goes first: a crop rect or a rotation is expressed
        // against the image as displayed, not as stored, so every later step
        // needs upright pixels to measure against.
        if orientation != .up {
            current = normalized(
                current.oriented(forExifOrientation: Int32(orientation.rawValue))
            )
            changed = true
        }

        for operation in operations {
            let size = current.extent.size

            switch operation {
            case .crop(let spec):
                guard let rect = spec.rect(for: size) else { continue }
                // CropSpec's origin is the top left; CoreImage's is the bottom
                // left, so the y coordinate has to be mirrored.
                let region = CGRect(
                    x: current.extent.minX + rect.minX,
                    y: current.extent.maxY - rect.maxY,
                    width: rect.width,
                    height: rect.height
                )
                current = normalized(current.cropped(to: region))
                changed = true

            case .rotate(let degrees):
                guard let turns = ImageOperation.quarterTurns(degrees) else {
                    throw ToolboxError.unsupportedRotation(degrees)
                }
                guard turns != 0 else { continue }
                current = normalized(current.transformed(by: Self.rotation(quarterTurns: turns)))
                changed = true

            case .flip(let horizontal, let vertical):
                guard horizontal || vertical else { continue }
                current = normalized(current.transformed(
                    by: CGAffineTransform(scaleX: horizontal ? -1 : 1, y: vertical ? -1 : 1)
                ))
                changed = true

            case .resize(let spec):
                guard let target = spec.target(for: size, allowUpscale: allowUpscale) else {
                    continue
                }
                current = try Self.scaled(current, to: target)
                changed = true
            }
        }

        // Every operation resolved to nothing for this particular image, so
        // there is no reason to pay for a render.
        guard changed else {
            return RenderedImage(image: image, orientationBaked: false, didTransform: false)
        }

        let bounds = CGRect(
            x: 0, y: 0,
            width: max(1, current.extent.width.rounded()),
            height: max(1, current.extent.height.rounded())
        )
        // Clamping before the final crop keeps a fractional extent from leaving
        // a transparent hairline along an edge.
        guard let rendered = context.createCGImage(
            normalized(current).clampedToExtent().cropped(to: bounds),
            from: bounds,
            format: .RGBA8,
            colorSpace: Self.outputColorSpace(for: image)
        ) else {
            throw ToolboxError.encodeFailed("bitmap")
        }
        return RenderedImage(
            image: rendered,
            orientationBaked: orientation != .up,
            didTransform: true
        )
    }

    /// Keeps the source's own colour space so a wide-gamut photo isn't quietly
    /// flattened to sRGB. Anything that isn't RGB can't describe the RGBA8
    /// buffer we render into, so those fall back to sRGB.
    private static func outputColorSpace(for image: CGImage) -> CGColorSpace {
        if let space = image.colorSpace, space.model == .rgb {
            return space
        }
        return CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
    }

    /// Moves the extent back to the origin so the next step measures against a
    /// clean coordinate space. Purely lazy — CoreImage folds the transforms
    /// together and none of this allocates.
    private static func normalized(_ image: CIImage) -> CIImage {
        let extent = image.extent
        guard extent.minX != 0 || extent.minY != 0 else { return image }
        return image.transformed(
            by: CGAffineTransform(translationX: -extent.minX, y: -extent.minY)
        )
    }

    /// Exact integer matrices rather than `CGAffineTransform(rotationAngle:)`:
    /// cos and sin of π/2 are not exactly 0 in floating point, and the residue
    /// turns a free relabelling of pixels into a resample.
    private static func rotation(quarterTurns: Int) -> CGAffineTransform {
        switch quarterTurns {
        case 1: return CGAffineTransform(a: 0, b: -1, c: 1, d: 0, tx: 0, ty: 0)
        case 2: return CGAffineTransform(a: -1, b: 0, c: 0, d: -1, tx: 0, ty: 0)
        case 3: return CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: 0, ty: 0)
        default: return .identity
        }
    }

    /// Lanczos rather than a plain affine scale: an affine transform samples
    /// bilinearly, which visibly softens a large downscale.
    private static func scaled(_ image: CIImage, to target: CGSize) throws -> CIImage {
        let extent = image.extent
        guard extent.width >= 1, extent.height >= 1 else {
            throw ToolboxError.encodeFailed("bitmap")
        }

        let verticalScale = target.height / extent.height
        let horizontalScale = target.width / extent.width

        let filter = CIFilter.lanczosScaleTransform()
        filter.inputImage = image
        filter.scale = Float(verticalScale)
        filter.aspectRatio = Float(horizontalScale / verticalScale)
        guard let output = filter.outputImage else {
            throw ToolboxError.encodeFailed("bitmap")
        }

        // Pin the result to whole pixels immediately so a following step, and
        // the reported pixel size, see exactly the size that was asked for.
        let box = CGRect(origin: .zero, size: target)
        return normalized(output).clampedToExtent().cropped(to: box)
    }
}
