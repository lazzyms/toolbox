import CoreGraphics
import Foundation
import ImageIO

/// The crop a user asked for, mapped onto the pipeline's `CropSpec`.
///
/// The pipeline already owns the two hard parts — applying EXIF orientation
/// before measuring, and clamping an out-of-bounds rect — so this layer carries
/// only the tool's vocabulary: an anchored aspect-ratio crop or a fixed pixel
/// rectangle, plus the fit check that makes a numeric crop over a smaller image
/// fail loudly instead of silently keeping whatever overlapped.
public enum CropOptions: Sendable, Equatable {
    /// Crop to the largest rect of this ratio that fits, positioned by anchor.
    case aspect(width: Int, height: Int, anchor: CropAnchor)
    /// Crop to a fixed pixel rectangle.
    case rect(x: Int, y: Int, width: Int, height: Int)

    /// The pipeline operation this describes.
    public var operation: ImageOperation {
        switch self {
        case .aspect(let width, let height, let anchor):
            return .crop(.aspect(width: width, height: height, anchor: anchor))
        case .rect(let x, let y, let width, let height):
            return .crop(.rect(x: x, y: y, width: width, height: height))
        }
    }

    /// Whether the fields describe a crop at all.
    ///
    /// Size-independent — a rect that's too big for one image is fine for a
    /// bigger one, so fit is a per-file check (`validationError`), not this.
    /// An aspect ratio of 0 or a zero-area rect means "nothing to crop".
    public var isValid: Bool {
        switch self {
        case .aspect(let width, let height, _):
            return width > 0 && height > 0
        case .rect(let x, let y, let width, let height):
            return x >= 0 && y >= 0 && width > 0 && height > 0
        }
    }

    /// Whether a source of `size` pixels is actually cropped by this.
    ///
    /// An aspect crop that already fills the whole image, or a rect covering
    /// it, resolves to no change and would waste a re-encode.
    public func isActive(for size: CGSize) -> Bool {
        pipelineSpec.rect(for: size) != nil
    }

    /// Why this crop can't apply to an image of `size` pixels, if it can't.
    ///
    /// Aspect crops always fit — they're sized to the image — so only pixel
    /// rects can fail here. A rect is refused when it's zero-area, starts
    /// outside the image, or hangs over an edge, so a batch of mismatched
    /// screenshots names the offender instead of producing something unexpected.
    public func validationError(for size: CGSize) -> ToolboxError? {
        guard case .rect(let x, let y, let width, let height) = self else { return nil }
        guard width > 0, height > 0 else {
            return .invalidCrop("A crop needs a positive width and height")
        }
        guard x >= 0, y >= 0 else {
            return .invalidCrop("The crop starts before the image's top-left corner")
        }
        guard Double(x) + Double(width) <= size.width,
              Double(y) + Double(height) <= size.height else {
            return .invalidCrop(
                "A \(width)×\(height) crop at \(x),\(y) doesn't fit "
                    + "the \(Int(size.width))×\(Int(size.height)) image"
            )
        }
        return nil
    }

    /// The image's pixel size as the user sees it, EXIF orientation applied.
    ///
    /// A photo stored 40×20 with a tag that rotates it a quarter turn is 20×40
    /// on screen, and the pipeline measures crops against the image as
    /// displayed — so this is the size a fit check must compare against.
    public static func pixelSize(of url: URL) -> CGSize? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = (properties[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue,
              let height = (properties[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue
        else { return nil }
        // Orientations 5–8 swap the stored sides.
        let orientation = (properties[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1
        let swapsAxes = (5...8).contains(orientation)
        return swapsAxes ? CGSize(width: height, height: width) : CGSize(width: width, height: height)
    }

    private var pipelineSpec: CropSpec {
        switch self {
        case .aspect(let width, let height, let anchor):
            return .aspect(width: width, height: height, anchor: anchor)
        case .rect(let x, let y, let width, let height):
            return .rect(x: x, y: y, width: width, height: height)
        }
    }
}
