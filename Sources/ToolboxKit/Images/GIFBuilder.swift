import CoreGraphics
import Foundation
import ImageIO

/// The two GIF tools: several stills into one animation, and an animation back
/// into its frames.
///
/// Both directions are thin wrappers over `ImageFrameSequence`, which already
/// knows how to read and write multi-frame files with their timing. What this
/// type adds is the tool-shaped API: create decodes each still upright and
/// optionally caps its size, extract hands back every frame as its own PNG.
public enum GIFBuilder {

    // MARK: Create

    public struct CreateOptions: Sendable {
        /// Seconds each frame is shown. Viewers clamp sub-0.02s values
        /// unpredictably, so anything below that floor is raised to it.
        public var frameDelay: Double
        /// 0 means loop forever, matching `kCGImagePropertyGIFLoopCount`.
        public var loopCount: Int
        /// Longest side every frame is capped to, so a drop of full-resolution
        /// photos can't silently become a multi-hundred-MB file. nil leaves
        /// frames at their source size.
        public var maxDimension: Int?
        public var location: OutputLocation

        public init(
            frameDelay: Double = 0.1,
            loopCount: Int = 0,
            maxDimension: Int? = 1280,
            location: OutputLocation = .alongsideInput
        ) {
            self.frameDelay = frameDelay
            self.loopCount = loopCount
            self.maxDimension = maxDimension
            self.location = location
        }
    }

    public struct CreateResult: Sendable {
        public let output: URL
        public let frameCount: Int
        public let framesPerSecond: Double
    }

    /// Several stills become one animated GIF. Queue order is frame order.
    public static func createGIF(
        from inputs: [URL], options: CreateOptions
    ) throws -> CreateResult {
        guard !inputs.isEmpty else {
            throw ToolboxError.invalidGIFOptions("Add at least one image to animate.")
        }

        for input in inputs where !ImageFormat.isReadable(input) {
            throw ToolboxError.unsupportedInput(input.pathExtension)
        }

        let delay = max(options.frameDelay, 0.02)
        let frames = try inputs.map { input in
            ImageFrameSequence.Frame(
                image: try decodedFrame(input, maxDimension: options.maxDimension),
                delay: delay
            )
        }

        let output = OutputNaming.destination(
            for: inputs[0],
            in: options.location,
            suffix: "-animated",
            extension: "gif"
        )

        let sequence = ImageFrameSequence(
            container: .gif,
            frames: frames,
            loopCount: options.loopCount,
            didTransform: false
        )
        try sequence.write(to: output, quality: 1)

        return CreateResult(
            output: output,
            frameCount: frames.count,
            framesPerSecond: 1 / delay
        )
    }

    // MARK: Extract

    public struct ExtractOptions: Sendable {
        public var location: OutputLocation

        public init(location: OutputLocation = .alongsideInput) {
            self.location = location
        }
    }

    public struct Extraction: Sendable {
        public let outputs: [URL]
        /// Per-frame delay in the order the frames appear.
        public let delays: [Double]
        public let loopCount: Int

        public var totalDuration: Double { delays.reduce(0, +) }
    }

    /// Every frame of an animation becomes its own PNG.
    ///
    /// Frames come back fully composited — a GIF that stores frame 2 as "only
    /// the bits that changed" still yields a complete 64×48 frame, not a
    /// fragment. ImageIO does that compositing during the decode.
    public static func extractFrames(
        from input: URL, options: ExtractOptions
    ) throws -> Extraction {
        let sourceOptions: [CFString: Any] = [kCGImageSourceShouldCache: false]
        guard let source = CGImageSourceCreateWithURL(
            input as CFURL, sourceOptions as CFDictionary
        ) else {
            throw ToolboxError.decodeFailed(input)
        }
        guard CGImageSourceGetCount(source) > 0 else {
            throw ToolboxError.decodeFailed(input)
        }

        // A passthrough transform: the point of extraction is the frames as
        // they are, not a transformation of them. `read` still supplies the
        // frame/delay/loop reading, so timing can't drift from the rest of the
        // codebase.
        let transform = ImageTransform(
            operations: [], sourceSize: .zero, orientation: .up, allowUpscale: false
        )
        guard let sequence = try ImageFrameSequence.read(
            from: source, input: input, requestedFormat: nil, transform: transform
        ) else {
            throw ToolboxError.notAnimated(input)
        }

        var outputs: [URL] = []
        outputs.reserveCapacity(sequence.frames.count)
        for (index, frame) in sequence.frames.enumerated() {
            let output = OutputNaming.destination(
                for: input,
                in: options.location,
                suffix: "-frame-\(index + 1)",
                extension: "png"
            )
            try writePNG(frame.image, to: output)
            outputs.append(output)
        }

        return Extraction(
            outputs: outputs,
            delays: sequence.frames.map(\.delay),
            loopCount: sequence.loopCount
        )
    }

    // MARK: - Helpers

    /// Decodes one frame upright, capping its longest side.
    ///
    /// The thumbnail route decodes, applies the EXIF orientation and scales in
    /// one step, so a phone photo comes out the way it is displayed and a
    /// too-large drop is bounded before it ever reaches the GIF encoder.
    private static func decodedFrame(_ input: URL, maxDimension: Int?) throws -> CGImage {
        let sourceOptions: [CFString: Any] = [kCGImageSourceShouldCache: false]
        guard let source = CGImageSourceCreateWithURL(
            input as CFURL, sourceOptions as CFDictionary
        ), CGImageSourceGetCount(source) > 0 else {
            throw ToolboxError.decodeFailed(input)
        }

        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            // A cap above the source's own longest side never binds.
            kCGImageSourceThumbnailMaxPixelSize: maxDimension ?? 100_000,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
        else {
            throw ToolboxError.decodeFailed(input)
        }
        return image
    }

    private static func writePNG(_ image: CGImage, to output: URL) throws {
        guard let destination = CGImageDestinationCreateWithURL(
            output as CFURL, "public.png" as CFString, 1, nil
        ) else {
            throw ToolboxError.writeFailed(output)
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.encodeFailed("PNG")
        }
    }
}
