import CoreGraphics
import Foundation
import ImageIO

/// Every frame of an animated or multi-page image, with the timing needed to
/// replay it.
///
/// `ImageProcessor` reads one of these instead of frame 0 whenever an input
/// holds more than one frame, so an animated GIF survives convert, compress and
/// resize rather than coming back as a still image that reports success.
struct ImageFrameSequence {

    struct Frame {
        let image: CGImage
        /// Seconds this frame is shown. 0 in containers that hold pages rather
        /// than an animation, such as multi-page TIFF.
        let delay: Double
    }

    /// The multi-frame containers ImageIO understands, and where each one keeps
    /// its timing.
    ///
    /// Formats whose extra indices aren't animation frames are deliberately
    /// absent: RAW files expose embedded previews as extra indices, so treating
    /// every multi-index file as an animation would fail conversions that have
    /// always worked. Those keep the first-image behaviour.
    enum Container: CaseIterable {
        case gif
        /// Animated PNG. Still PNGs never report more than one frame.
        case apng
        /// HEIF image sequence — the animated sibling of HEIC.
        case heics
        /// Pages rather than frames: preserved, but there is no timing to keep.
        case tiff
        /// Readable only. ImageIO has never shipped an animated WebP encoder,
        /// so these can be decoded but not written back.
        case webp

        init?(typeIdentifier: String) {
            guard let match = Self.allCases.first(where: { $0.typeIdentifier == typeIdentifier })
            else { return nil }
            self = match
        }

        var typeIdentifier: String {
            switch self {
            case .gif: return "com.compuserve.gif"
            case .apng: return "public.png"
            case .heics: return "public.heics"
            case .tiff: return "public.tiff"
            case .webp: return "org.webmproject.webp"
            }
        }

        /// Used for the output name, so the extension can never disagree with
        /// the bytes even when the input was misnamed.
        var fileExtension: String {
            switch self {
            case .gif: return "gif"
            case .apng: return "png"
            case .heics: return "heics"
            case .tiff: return "tiff"
            case .webp: return "webp"
            }
        }

        var displayName: String {
            switch self {
            case .gif: return "GIF"
            case .apng: return "PNG"
            case .heics: return "HEICS"
            case .tiff: return "TIFF"
            case .webp: return "WebP"
            }
        }

        /// Whether this Mac can write the frames back. WebP is excluded outright
        /// rather than by capability check: a Mac that can encode a still WebP
        /// still cannot encode an animated one.
        var canWriteFrames: Bool {
            guard self != .webp else { return false }
            return (CGImageDestinationCopyTypeIdentifiers() as? [String])?
                .contains(typeIdentifier) ?? false
        }

        /// The only container here with a quality dial — HEICS frames are HEVC.
        /// GIF is palette-based, and TIFF pages and APNG frames are lossless.
        var supportsQuality: Bool { self == .heics }

        /// The dictionary holding per-frame timing, and the keys inside it.
        /// `nil` for containers with no concept of timing.
        var timingDictionaryKey: CFString? {
            switch self {
            case .gif: return kCGImagePropertyGIFDictionary
            case .apng: return kCGImagePropertyPNGDictionary
            case .heics: return kCGImagePropertyHEICSDictionary
            case .webp: return kCGImagePropertyWebPDictionary
            case .tiff: return nil
            }
        }

        var delayTimeKey: CFString? {
            switch self {
            case .gif: return kCGImagePropertyGIFDelayTime
            case .apng: return kCGImagePropertyAPNGDelayTime
            case .heics: return kCGImagePropertyHEICSDelayTime
            case .webp: return kCGImagePropertyWebPDelayTime
            case .tiff: return nil
            }
        }

        /// The delay as the file actually states it. ImageIO's clamped value
        /// applies a browser-compatible 0.1s floor.
        var unclampedDelayTimeKey: CFString? {
            switch self {
            case .gif: return kCGImagePropertyGIFUnclampedDelayTime
            case .apng: return kCGImagePropertyAPNGUnclampedDelayTime
            case .heics: return kCGImagePropertyHEICSUnclampedDelayTime
            case .webp: return kCGImagePropertyWebPUnclampedDelayTime
            case .tiff: return nil
            }
        }

        var loopCountKey: CFString? {
            switch self {
            case .gif: return kCGImagePropertyGIFLoopCount
            case .apng: return kCGImagePropertyAPNGLoopCount
            case .heics: return kCGImagePropertyHEICSLoopCount
            case .webp: return kCGImagePropertyWebPLoopCount
            case .tiff: return nil
            }
        }
    }

    let container: Container
    let frames: [Frame]
    /// 0 means "repeat forever", matching `kCGImagePropertyGIFLoopCount`.
    let loopCount: Int
    /// Whether the operations actually changed the frames, which is what tells
    /// the caller's no-inflation guard that comparing sizes is meaningless.
    let didTransform: Bool

    var fileExtension: String { container.fileExtension }

    /// The frame a caller reports dimensions from.
    var firstImage: CGImage { frames[0].image }

    // MARK: - Reading

    /// Decodes every frame of a multi-frame input, applying `transform` to each.
    ///
    /// Returns `nil` for anything that is a single image — the caller's existing
    /// one-frame path handles those. Throws rather than returning frame 0 when
    /// the frames cannot survive the requested output.
    static func read(
        from source: CGImageSource,
        input: URL,
        requestedFormat: ImageFormat?,
        transform: ImageTransform
    ) throws -> ImageFrameSequence? {
        let count = CGImageSourceGetCount(source)
        guard count > 1,
              let typeIdentifier = CGImageSourceGetType(source) as String?,
              let container = Container(typeIdentifier: typeIdentifier)
        else { return nil }

        // Changing container loses either the frames (JPEG, still PNG) or their
        // timing (TIFF), and quietly handing back frame 0 is the bug this type
        // exists to fix. Keeping the input's own format — what resize and
        // lossless compress ask for — is the case that can be honoured.
        if let requestedFormat, requestedFormat.utType.identifier != typeIdentifier {
            throw ToolboxError.wouldDropFrames(
                input, frames: count, format: requestedFormat.displayName
            )
        }

        guard container.canWriteFrames else {
            throw ToolboxError.cannotWriteFrames(
                input, frames: count, format: container.displayName
            )
        }

        // Every frame goes through the same transform as a still image would,
        // so animated and single-frame inputs can't drift apart.
        var frames: [Frame] = []
        frames.reserveCapacity(count)
        var didTransform = false
        for index in 0..<count {
            let rendered = try transform.decode(from: source, at: index, input: input)
            if index == 0 { didTransform = rendered.didTransform }
            frames.append(
                Frame(
                    image: rendered.image,
                    delay: delay(from: source, at: index, container: container)
                )
            )
        }

        return ImageFrameSequence(
            container: container,
            frames: frames,
            loopCount: loopCount(from: source, container: container),
            didTransform: didTransform
        )
    }

    private static func delay(
        from source: CGImageSource, at index: Int, container: Container
    ) -> Double {
        guard let dictionaryKey = container.timingDictionaryKey,
              let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil)
                as? [CFString: Any],
              let timing = properties[dictionaryKey] as? [CFString: Any]
        else { return 0 }

        // Prefer the unclamped value, which is what the file says. A zero there
        // means "as fast as possible", and the clamped value is ImageIO's
        // browser-compatible answer to that, so it's the better thing to write.
        if let key = container.unclampedDelayTimeKey,
           let unclamped = (timing[key] as? NSNumber)?.doubleValue, unclamped > 0 {
            return unclamped
        }
        if let key = container.delayTimeKey,
           let clamped = (timing[key] as? NSNumber)?.doubleValue {
            return clamped
        }
        return 0
    }

    private static func loopCount(from source: CGImageSource, container: Container) -> Int {
        guard let dictionaryKey = container.timingDictionaryKey,
              let loopCountKey = container.loopCountKey,
              let properties = CGImageSourceCopyProperties(source, nil) as? [CFString: Any],
              let timing = properties[dictionaryKey] as? [CFString: Any],
              let value = (timing[loopCountKey] as? NSNumber)?.intValue
        else { return 0 }
        return value
    }

    // MARK: - Writing

    /// Writes every frame plus its timing.
    ///
    /// Nothing is copied from the source's metadata: the frames are re-encoded
    /// from pixels, GIF carries no EXIF worth keeping, and leaving the tags
    /// behind also means an orientation tag can't ride along and rotate an
    /// already-upright frame a second time.
    func write(to output: URL, quality: Double) throws {
        guard let destination = CGImageDestinationCreateWithURL(
            output as CFURL, container.typeIdentifier as CFString, frames.count, nil
        ) else {
            throw ToolboxError.writeFailed(output)
        }

        if let dictionaryKey = container.timingDictionaryKey,
           let loopCountKey = container.loopCountKey {
            CGImageDestinationSetProperties(
                destination,
                [dictionaryKey: [loopCountKey: loopCount]] as CFDictionary
            )
        }

        for frame in frames {
            var properties: [CFString: Any] = [:]
            if container.supportsQuality {
                properties[kCGImageDestinationLossyCompressionQuality] =
                    min(max(quality, 0.0), 1.0)
            }
            if let dictionaryKey = container.timingDictionaryKey,
               let delayTimeKey = container.delayTimeKey {
                var timing: [CFString: Any] = [delayTimeKey: frame.delay]
                // Write both so a viewer reading either key gets the same answer.
                if let unclampedKey = container.unclampedDelayTimeKey {
                    timing[unclampedKey] = frame.delay
                }
                properties[dictionaryKey] = timing
            }
            CGImageDestinationAddImage(destination, frame.image, properties as CFDictionary)
        }

        guard CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.encodeFailed(container.displayName)
        }
    }
}
