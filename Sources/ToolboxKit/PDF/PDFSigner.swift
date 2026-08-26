import Foundation
import PDFKit
import CoreGraphics
import CoreText
import ImageIO

public struct SignOptions: Sendable {
    public enum Content: Sendable {
        /// A signature image, typically a transparent PNG scrawl.
        case image(URL)
        /// A name rendered in a script font — for when there is no image.
        case typedName(String)
    }

    public var content: Content
    /// Where the signature sits on the page. One of the nine corners/edges;
    /// signatures never tile, unlike watermarks.
    public var anchor: StampAnchor
    /// Points kept between the signature and the page edges.
    public var inset: Double
    /// Image mode only: signature width as a fraction of page width. Height
    /// follows the image's own aspect ratio, never stretched.
    public var widthFraction: Double
    /// Typed-name mode only: point size of the script font.
    public var fontSize: Double
    public var opacity: Double

    public init(
        content: Content,
        anchor: StampAnchor = .bottomRight,
        inset: Double = 24,
        widthFraction: Double = 0.28,
        fontSize: Double = 36,
        opacity: Double = 1.0
    ) {
        self.content = content
        self.anchor = anchor
        self.inset = inset
        self.widthFraction = widthFraction
        self.fontSize = fontSize
        self.opacity = opacity
    }
}

/// Stamps a visual signature onto PDF pages: an image or a typed name drawn
/// over the page content.
///
/// Deliberately *not* cryptographic signing — no certificate, no integrity
/// proof. It flattens the signature into the page stream (not an annotation),
/// so it can't be dragged off in Preview, exactly like a scanned signature.
public enum PDFSigner {
    /// The measured ink: either a decoded image, or the typographic metrics of
    /// the typed name's rendered line. Measured once, before any page draws.
    private enum Ink {
        case image(CGImage)
        case typed(width: CGFloat, ascent: CGFloat, descent: CGFloat)
    }

    /// Script font shipped with macOS for the typed-name fallback.
    private static let scriptFontName = "SnellRoundhand"

    public static func apply(
        _ options: SignOptions,
        to input: URL,
        pageRangeText: String?,
        to location: OutputLocation
    ) throws -> URL {
        if PDFUnlocker.isEncrypted(input) {
            throw ToolboxError.passwordProtected(input)
        }
        // Resolve and validate the ink before touching disk: a bad signature
        // should fail fast, not leave a half-written output behind.
        let ink = try resolveInk(options)

        let doc = try PDFDocumentIO.open(input)
        let pageCount = doc.pageCount
        guard pageCount > 0 else {
            throw ToolboxError.notAPDF(input)
        }

        let signedPages: Set<Int>
        if let rangeText = pageRangeText, !rangeText.trimmingCharacters(in: .whitespaces).isEmpty {
            signedPages = Set(try PageRange.parse(rangeText, pageCount: pageCount))
        } else {
            signedPages = Set(0..<pageCount)
        }

        let output = OutputNaming.destination(for: input, in: location, suffix: "-signed", extension: "pdf")

        let firstBox = doc.page(at: 0)!.bounds(for: .mediaBox)
        var mediaBox = firstBox
        guard let context = CGContext(output as CFURL, mediaBox: &mediaBox, nil) else {
            throw ToolboxError.writeFailed(output)
        }

        for index in 0..<pageCount {
            guard let page = doc.page(at: index) else { continue }
            var box = page.bounds(for: .mediaBox)
            context.beginPage(mediaBox: &box)

            PDFPageReplay.replay(page: page, into: context)
            // A signature always goes over the content — never under, the way
            // a watermark can — and alpha stays scoped inside GraphicsStamp.
            if signedPages.contains(index) {
                stamp(ink, options, in: context, bounds: box)
            }
            context.endPage()
        }
        context.closePDF()
        return output
    }

    private static func resolveInk(_ options: SignOptions) throws -> Ink {
        switch options.content {
        case .image(let url):
            guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
                  let image = CGImageSourceCreateImageAtIndex(source, 0, nil), image.width > 0 else {
                throw ToolboxError.decodeFailed(url)
            }
            return .image(image)
        case .typedName(let name):
            let trimmed = name.trimmingCharacters(in: .whitespaces)
            guard !trimmed.isEmpty else {
                throw ToolboxError.emptySignature
            }
            let font = CTFontCreateWithName(scriptFontName as CFString, CGFloat(options.fontSize), nil)
            let attributes: [CFString: Any] = [
                kCTFontAttributeName: font,
                kCTForegroundColorAttributeName: CGColor(gray: 0, alpha: 1),
            ]
            guard let attributed = CFAttributedStringCreate(nil, trimmed as CFString, attributes as CFDictionary),
                  let line = CTLineCreateWithAttributedString(attributed) as CTLine? else {
                throw ToolboxError.emptySignature
            }
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            CTLineGetTypographicBounds(line, &ascent, &descent, nil)
            let width = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
            return .typed(width: width, ascent: ascent, descent: descent)
        }
    }

    private static func stamp(_ ink: Ink, _ options: SignOptions, in context: CGContext, bounds: CGRect) {
        switch ink {
        case .image(let image):
            // Height derives from the image's own aspect ratio so a scrawl
            // never comes out squashed, whatever its pixel dimensions.
            let width = bounds.width * CGFloat(options.widthFraction)
            let height = width * CGFloat(image.height) / CGFloat(image.width)
            let dest = destinationRect(
                size: CGSize(width: width, height: height),
                in: bounds, anchor: options.anchor, inset: CGFloat(options.inset)
            )
            // GraphicsStamp sizes an image stamp as pixel dimensions × scale
            // and puts its bottom-left corner on the given bounds' midY — so
            // the fraction carries the resizing, and the frame shifts down by
            // half the stamp height to land the ink exactly on `dest`.
            let frame = CGRect(
                x: dest.midX - dest.width / 2,
                y: dest.minY - dest.height / 2,
                width: dest.width,
                height: dest.height
            )
            GraphicsStamp.draw(
                image, in: context, bounds: frame, anchor: .center,
                scale: Double(dest.width / CGFloat(image.width)),
                opacity: options.opacity
            )

        case .typed(let width, let ascent, let descent):
            guard case .typedName(let name) = options.content else { return }
            let size = CGSize(width: width, height: ascent + descent)
            let dest = destinationRect(
                size: size,
                in: bounds, anchor: options.anchor, inset: CGFloat(options.inset)
            )
            let stamp = TextStamp(
                text: name.trimmingCharacters(in: .whitespaces),
                fontName: scriptFontName,
                size: .points(CGFloat(options.fontSize)),
                color: CGColor(gray: 0, alpha: 1),
                opacity: options.opacity
            )
            // Same idea as the image branch, calibrated for the text path:
            // GraphicsStamp puts the baseline at the frame's midY plus one
            // ascent, so shifting the frame down by (ascent − height ÷ 2)
            // drops the whole glyph box exactly onto `dest`.
            let frame = CGRect(
                x: dest.midX - dest.width / 2,
                y: dest.maxY - 2 * ascent - dest.height / 2,
                width: dest.width,
                height: dest.height
            )
            GraphicsStamp.draw(stamp, in: context, bounds: frame, anchor: .center)
        }
    }

    /// Resolves a nine-anchor choice plus inset into the exact rectangle the
    /// signature should occupy. CoreGraphics coordinates: origin bottom-left.
    private static func destinationRect(
        size: CGSize,
        in bounds: CGRect,
        anchor: StampAnchor,
        inset: CGFloat
    ) -> CGRect {
        let box = bounds.insetBy(dx: inset, dy: inset)
        let x: CGFloat
        let y: CGFloat
        switch anchor {
        case .topLeft, .top, .topRight:
            y = box.maxY - size.height
        case .left, .center, .right:
            y = box.midY - size.height / 2
        default:
            y = box.minY
        }
        switch anchor {
        case .topLeft, .left, .bottomLeft:
            x = box.minX
        case .top, .center, .bottom:
            x = box.midX - size.width / 2
        default:
            x = box.maxX - size.width
        }
        return CGRect(x: x, y: y, width: size.width, height: size.height)
    }
}
