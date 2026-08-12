/// How to rotate and/or mirror an image.
///
/// Rotation is expressed in whole degrees, clockwise, the way a user describes
/// it. Quarter turns are the supported range: the renderer's exact integer
/// matrices rotate without resampling, and anything that isn't a right angle
/// reaches the renderer and fails there with a reason rather than being
/// silently dropped.
public struct RotateSpec: Sendable, Equatable {
    /// Degrees clockwise. 0 means "no rotation".
    public var degrees: Int
    public var flipHorizontal: Bool
    public var flipVertical: Bool

    public init(degrees: Int = 0, flipHorizontal: Bool = false, flipVertical: Bool = false) {
        self.degrees = degrees
        self.flipHorizontal = flipHorizontal
        self.flipVertical = flipVertical
    }

    /// Whether running this on an image would actually change its pixels.
    /// A whole number of turns normalises away, so a 360° rotation isn't work.
    public var isActive: Bool {
        ImageOperation.normalisedDegrees(degrees) != 0 || flipHorizontal || flipVertical
    }

    /// The pipeline steps this describes, in the order they are applied.
    /// Rotation first, then mirroring — the order a user reads in "rotate then
    /// flip", and the one the renderer turns into a single CoreImage graph.
    public var operations: [ImageOperation] {
        var operations: [ImageOperation] = []
        if ImageOperation.normalisedDegrees(degrees) != 0 {
            operations.append(.rotate(degrees: degrees))
        }
        if flipHorizontal || flipVertical {
            operations.append(.flip(horizontal: flipHorizontal, vertical: flipVertical))
        }
        return operations
    }
}
