import CoreGraphics
import CoreText
import Foundation
import ImageIO

public struct ImageWatermarkOptions: Sendable {
    public enum Content: Sendable {
        case text(String)
        case image(URL)
    }

    public var content: Content
    /// Text mode only. `.points` is an absolute size; `.fraction` sizes the font
    /// as a fraction of the image's width, so a batch of mixed dimensions gets
    /// consistently scaled stamps instead of one huge and one invisible.
    public var fontSize: StampSize
    public var color: CGColor
    public var opacity: Double
    /// Text mode only: degrees clockwise; the classic diagonal stamp is ±45.
    public var rotationDegrees: Double
    /// Where the stamp lands. `.tiled(spacing)` repeats it across the canvas.
    public var anchor: StampAnchor
    /// Image mode only: stamp width as a fraction of the image's width.
    public var imageScale: Double

    public init(
        content: Content,
        fontSize: StampSize = .points(48),
        color: CGColor = CGColor(gray: 0.5, alpha: 1),
        opacity: Double = 0.3,
        rotationDegrees: Double = -45,
        anchor: StampAnchor = .center,
        imageScale: Double = 0.4
    ) {
        self.content = content
        self.fontSize = fontSize
        self.color = color
        self.opacity = opacity
        self.rotationDegrees = rotationDegrees
        self.anchor = anchor
        self.imageScale = imageScale
    }
}

public enum ImageWatermarker {
    public static func apply(
        _ options: ImageWatermarkOptions,
        to input: URL,
        destination location: OutputLocation,
        quality: Double = 0.8
    ) throws -> URL {
        if case .text(let text) = options.content,
           text.trimmingCharacters(in: .whitespaces).isEmpty {
            throw ToolboxError.emptyWatermark
        }

        // kCGImageSourceShouldCache false: each pixel buffer is touched once,
        // so caching only inflates memory during batch runs.
        let sourceOptions: [CFString: Any] = [kCGImageSourceShouldCache: false]
        guard let source = CGImageSourceCreateWithURL(input as CFURL, sourceOptions as CFDictionary),
              CGImageSourceGetCount(source) > 0
        else {
            throw ToolboxError.decodeFailed(input)
        }

        let format = outputFormat(for: input, source: source)

        // A watermark rewrites the pixels as a still image; an animated input
        // must not come back as frame 0 wearing a success badge.
        let frameCount = CGImageSourceGetCount(source)
        if frameCount > 1 {
            throw ToolboxError.wouldDropFrames(input, frames: frameCount, format: format.displayName)
        }

        guard let base = uprightImage(at: input), base.width > 0, base.height > 0 else {
            throw ToolboxError.decodeFailed(input)
        }

        // The EXIF orientation has been spent by the decode, so the stamp's
        // "top" is the top the user sees — no orientation tag may be copied to
        // the output or the result would display rotated a second time.
        let width = CGFloat(base.width)
        let height = CGFloat(base.height)
        let canvas = CGRect(x: 0, y: 0, width: width, height: height)

        // Keep the source's RGB space (wide-gamut photos stay wide); greyscale
        // and CMYK can't back an RGBA bitmap, so those fall to sRGB.
        let space: CGColorSpace
        if let baseSpace = base.colorSpace, baseSpace.model == .rgb {
            space = baseSpace
        } else {
            space = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
        }
        guard let context = CGContext(
            data: nil, width: base.width, height: base.height,
            bitsPerComponent: 8, bytesPerRow: 0, space: space,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else {
            throw ToolboxError.encodeFailed("bitmap")
        }

        context.draw(base, in: canvas)

        let metrics = measure(options, imageWidth: width)
        draw(options, in: context, canvas: canvas, metrics: metrics)

        guard let stamped = context.makeImage() else {
            throw ToolboxError.encodeFailed(format.displayName)
        }

        let output = OutputNaming.destination(
            for: input, in: location, suffix: "-watermarked", extension: format.fileExtension
        )
        guard let destination = CGImageDestinationCreateWithURL(
            output as CFURL, format.utType.identifier as CFString, 1, nil
        ) else {
            throw ToolboxError.writeFailed(output)
        }

        var properties: [CFString: Any] = [:]
        if format.supportsQuality {
            properties[kCGImageDestinationLossyCompressionQuality] = min(max(quality, 0.0), 1.0)
        }
        // No metadata is carried over: the pixels are new and upright, so any
        // copied orientation tag would rotate them on display.
        CGImageDestinationAddImage(destination, stamped, properties.isEmpty ? nil : properties as CFDictionary)
        guard CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.encodeFailed(format.displayName)
        }
        return output
    }

    /// The input's own container when this Mac can write it, else PNG — WebP,
    /// for one, has never had an ImageIO encoder.
    private static func outputFormat(for input: URL, source: CGImageSource) -> ImageFormat {
        let inferred: ImageFormat?
        if let type = CGImageSourceGetType(source) as String?,
           let match = ImageFormat.allCases.first(where: { $0.utType.identifier == type }) {
            inferred = match
        } else {
            let ext = input.pathExtension.lowercased()
            inferred = ImageFormat.allCases.first { $0.fileExtension == ext }
        }
        guard let match = inferred, match.canEncode else { return .png }
        return match
    }

    /// How big the stamp renders, plus how far GraphicsStamp raises a text
    /// baseline above the bottom of the box it is handed — its internal layout
    /// adds the full typographic height, which placement has to cancel out so
    /// the visible ink lands in the rectangle the anchor chose.
    private struct StampMetrics {
        var size: CGSize
        var baselineLift: CGFloat
    }

    private static func measure(_ options: ImageWatermarkOptions, imageWidth: CGFloat) -> StampMetrics {
        switch options.content {
        case .text(let text):
            let fontSize = resolvedFontSize(options.fontSize, imageWidth: imageWidth)
            let font = CTFontCreateWithName("Helvetica" as CFString, fontSize, nil)
            let attributes: [CFString: Any] = [
                kCTFontAttributeName: font,
                kCTForegroundColorAttributeName: options.color,
            ]
            guard let attributed = CFAttributedStringCreate(nil, text as CFString, attributes as CFDictionary),
                  let line = CTLineCreateWithAttributedString(attributed) as CTLine?
            else {
                return StampMetrics(size: CGSize(width: imageWidth / 4, height: imageWidth / 4), baselineLift: 0)
            }
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            CTLineGetTypographicBounds(line, &ascent, &descent, nil)
            let lineWidth = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
            return StampMetrics(
                size: CGSize(width: lineWidth, height: ascent + descent),
                baselineLift: ascent
            )
        case .image(let url):
            guard let image = loadImage(url), image.width > 0 else {
                let fallback = imageWidth * CGFloat(options.imageScale)
                return StampMetrics(size: CGSize(width: fallback, height: fallback), baselineLift: 0)
            }
            let stampWidth = imageWidth * CGFloat(options.imageScale)
            return StampMetrics(
                size: CGSize(width: stampWidth, height: stampWidth * CGFloat(image.height) / CGFloat(image.width)),
                baselineLift: 0
            )
        }
    }

    /// Resolved once per image against the full canvas width — not per tile —
    /// so every repeat of a tiled stamp comes out the same size.
    private static func resolvedFontSize(_ size: StampSize, imageWidth: CGFloat) -> CGFloat {
        switch size {
        case .points(let pt): return pt
        case .fraction(let f): return max(1, imageWidth * CGFloat(f))
        }
    }

    private static func draw(_ options: ImageWatermarkOptions, in context: CGContext, canvas: CGRect, metrics: StampMetrics) {
        switch options.anchor {
        case .tiled(let spacing):
            let strideWidth = metrics.size.width + CGFloat(spacing)
            let strideHeight = metrics.size.height + CGFloat(spacing)
            var y = canvas.minY
            while y < canvas.maxY {
                var x = canvas.minX
                while x < canvas.maxX {
                    let cell = CGRect(x: x, y: y, width: strideWidth, height: strideHeight).intersection(canvas)
                    if !cell.isEmpty {
                        drawOnce(options, in: context, inkBox: centred(metrics.size, in: cell), canvasWidth: canvas.width, metrics: metrics)
                    }
                    x += strideWidth
                }
                y += strideHeight
            }
        default:
            drawOnce(options, in: context, inkBox: placed(metrics.size, anchor: options.anchor, in: canvas), canvasWidth: canvas.width, metrics: metrics)
        }
    }

    /// The rectangle the stamp's ink occupies, honouring the anchor in the
    /// coordinates the user sees (top of the image = large y here, because
    /// CoreGraphics user space runs bottom-up).
    private static func placed(_ size: CGSize, anchor: StampAnchor, in canvas: CGRect) -> CGRect {
        // Margin keeps corner stamps off the very edge, scaled with the canvas
        // so it reads the same on a thumbnail as on a poster.
        let margin = min(canvas.width, canvas.height) * 0.04

        func clamp(_ point: CGPoint) -> CGPoint {
            CGPoint(
                x: min(max(point.x, 0), max(0, canvas.width - size.width)),
                y: min(max(point.y, 0), max(0, canvas.height - size.height))
            )
        }

        let x: CGFloat
        switch anchor {
        case .topLeft, .left, .bottomLeft: x = margin
        case .top, .center, .bottom: x = (canvas.width - size.width) / 2
        default: x = canvas.width - margin - size.width
        }

        let y: CGFloat
        switch anchor {
        case .topLeft, .top, .topRight: y = canvas.height - margin - size.height
        case .left, .center, .right: y = (canvas.height - size.height) / 2
        default: y = margin
        }

        return CGRect(origin: clamp(CGPoint(x: x, y: y)), size: size)
    }

    private static func centred(_ size: CGSize, in cell: CGRect) -> CGRect {
        CGRect(
            x: cell.midX - size.width / 2,
            y: cell.midY - size.height / 2,
            width: size.width,
            height: size.height
        )
    }

    private static func drawOnce(
        _ options: ImageWatermarkOptions,
        in context: CGContext,
        inkBox: CGRect,
        canvasWidth: CGFloat,
        metrics: StampMetrics
    ) {
        switch options.content {
        case .text(let text):
            // Drop the box by the baseline lift so the glyph bottoms — not the
            // typographic origin — sit where the anchor put them.
            var stampBox = inkBox
            stampBox.origin.y -= metrics.baselineLift
            let stamp = TextStamp(
                text: text,
                fontName: "Helvetica",
                size: .points(resolvedFontSize(options.fontSize, imageWidth: canvasWidth)),
                color: options.color,
                opacity: options.opacity,
                rotationDegrees: options.rotationDegrees
            )
            // The ink box already carries the user's anchor choice; pinning
            // GraphicsStamp to its own .bottomLeft gives a deterministic origin
            // instead of its .center, which parks the stamp's typographic
            // origin rather than its visible extent at the midpoint.
            GraphicsStamp.draw(stamp, in: context, bounds: stampBox, anchor: .bottomLeft)
        case .image(let url):
            guard let image = loadImage(url), image.width > 0 else { return }
            let scale = inkBox.width / CGFloat(image.width)
            GraphicsStamp.draw(image, in: context, bounds: inkBox, anchor: .bottomLeft, scale: Double(scale), opacity: options.opacity)
        }
    }

    private static func loadImage(_ url: URL) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(source, 0, nil)
    }

    /// Decodes with any EXIF orientation already applied — the same
    /// `CGImageSourceCreateThumbnailAtIndex` transform trick ImagesToPDF uses,
    /// because a bitmap canvas has no orientation tag either.
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
}
