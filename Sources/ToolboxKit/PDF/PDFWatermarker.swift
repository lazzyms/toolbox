import Foundation
import PDFKit
import CoreGraphics
import CoreText
import ImageIO

public struct WatermarkOptions: Sendable {
    public enum Content: Sendable {
        case text(String)
        case image(URL)
    }

    public var content: Content
    /// Text mode only: point size of the stamp.
    public var fontSize: Double
    public var color: CGColor
    public var opacity: Double
    /// Text mode only: degrees clockwise; the classic diagonal stamp is ±45.
    public var rotationDegrees: Double
    /// Where the stamp lands. `.tiled(spacing)` repeats it across the page.
    public var anchor: StampAnchor
    /// Draw beneath the page content so body text stays readable on top.
    public var underContent: Bool
    /// Image mode only: stamp width as a fraction of page width.
    public var imageScale: Double

    public init(
        content: Content,
        fontSize: Double = 48,
        color: CGColor = CGColor(gray: 0.5, alpha: 1),
        opacity: Double = 0.3,
        rotationDegrees: Double = -45,
        anchor: StampAnchor = .center,
        underContent: Bool = false,
        imageScale: Double = 0.4
    ) {
        self.content = content
        self.fontSize = fontSize
        self.color = color
        self.opacity = opacity
        self.rotationDegrees = rotationDegrees
        self.anchor = anchor
        self.underContent = underContent
        self.imageScale = imageScale
    }
}

public enum PDFWatermarker {
    public static func apply(
        _ options: WatermarkOptions,
        to input: URL,
        pageRangeText: String?,
        to location: OutputLocation
    ) throws -> URL {
        if PDFUnlocker.isEncrypted(input) {
            throw ToolboxError.passwordProtected(input)
        }
        if case .text(let text) = options.content,
           text.trimmingCharacters(in: .whitespaces).isEmpty {
            throw ToolboxError.emptyWatermark
        }

        let doc = try PDFDocumentIO.open(input)
        let pageCount = doc.pageCount
        guard pageCount > 0 else {
            throw ToolboxError.notAPDF(input)
        }

        let stampedPages: Set<Int>
        if let rangeText = pageRangeText, !rangeText.trimmingCharacters(in: .whitespaces).isEmpty {
            stampedPages = Set(try PageRange.parse(rangeText, pageCount: pageCount))
        } else {
            stampedPages = Set(0..<pageCount)
        }

        let output = OutputNaming.destination(for: input, in: location, suffix: "-watermarked", extension: "pdf")

        // Measure once: tiling needs each stamp's natural size up front.
        let stampSize = measure(options, pageWidth: doc.page(at: 0)!.bounds(for: .mediaBox).width)

        let firstBox = doc.page(at: 0)!.bounds(for: .mediaBox)
        var mediaBox = firstBox
        guard let context = CGContext(output as CFURL, mediaBox: &mediaBox, nil) else {
            throw ToolboxError.writeFailed(output)
        }

        for index in 0..<pageCount {
            guard let page = doc.page(at: index) else { continue }
            var box = page.bounds(for: .mediaBox)
            context.beginPage(mediaBox: &box)

            let stampHere = stampedPages.contains(index)
            // Under-content draws before the replay so page content covers it;
            // over-content draws after. Either way the alpha is scoped to this
            // stamp's own save/restore inside GraphicsStamp.
            if stampHere && options.underContent {
                draw(options, in: context, bounds: box, stampSize: stampSize)
            }
            PDFPageReplay.replay(page: page, into: context)
            if stampHere && !options.underContent {
                draw(options, in: context, bounds: box, stampSize: stampSize)
            }
            context.endPage()
        }
        context.closePDF()
        return output
    }

    private static func measure(_ options: WatermarkOptions, pageWidth: CGFloat) -> CGSize {
        switch options.content {
        case .text(let text):
            let font = CTFontCreateWithName("Helvetica" as CFString, CGFloat(options.fontSize), nil)
            let attributes: [CFString: Any] = [
                kCTFontAttributeName: font,
                kCTForegroundColorAttributeName: options.color,
            ]
            guard let attributed = CFAttributedStringCreate(nil, text as CFString, attributes as CFDictionary),
                  let line = CTLineCreateWithAttributedString(attributed) as CTLine? else {
                return CGSize(width: CGFloat(options.fontSize), height: CGFloat(options.fontSize))
            }
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            CTLineGetTypographicBounds(line, &ascent, &descent, nil)
            let width = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
            return CGSize(width: width, height: ascent + descent)
        case .image(let url):
            guard let image = loadImage(url) else {
                return CGSize(width: pageWidth * CGFloat(options.imageScale),
                              height: pageWidth * CGFloat(options.imageScale))
            }
            let width = pageWidth * CGFloat(options.imageScale)
            return CGSize(width: width, height: width * CGFloat(image.height) / CGFloat(image.width))
        }
    }

    private static func draw(_ options: WatermarkOptions, in context: CGContext, bounds: CGRect, stampSize: CGSize) {
        switch options.anchor {
        case .tiled(let spacing):
            let strideWidth = stampSize.width + CGFloat(spacing)
            let strideHeight = stampSize.height + CGFloat(spacing)
            var y = bounds.minY
            while y < bounds.maxY {
                var x = bounds.minX
                while x < bounds.maxX {
                    let cell = CGRect(x: x, y: y, width: strideWidth, height: strideHeight)
                    drawOnce(options, in: context, bounds: cell.intersection(bounds), stampSize: stampSize)
                    x += strideWidth
                }
                y += strideHeight
            }
        default:
            drawOnce(options, in: context, bounds: bounds, stampSize: stampSize)
        }
    }

    private static func drawOnce(_ options: WatermarkOptions, in context: CGContext, bounds: CGRect, stampSize: CGSize) {
        switch options.content {
        case .text(let text):
            let stamp = TextStamp(
                text: text,
                fontName: "Helvetica",
                size: .points(CGFloat(options.fontSize)),
                color: options.color,
                opacity: options.opacity,
                rotationDegrees: options.rotationDegrees
            )
            GraphicsStamp.draw(stamp, in: context, bounds: bounds, anchor: .center)
        case .image(let url):
            guard let image = loadImage(url), image.width > 0 else { return }
            let scale = stampSize.width / CGFloat(image.width)
            GraphicsStamp.draw(image, in: context, bounds: bounds, anchor: .center, scale: Double(scale), opacity: options.opacity)
        }
    }

    private static func loadImage(_ url: URL) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(source, 0, nil)
    }
}
