import AppKit
import CoreGraphics
import CoreText
import Foundation
import ImageIO
import PDFKit
import UniformTypeIdentifiers
@testable import ToolboxKit

/// Builds real files on disk so tests exercise the actual ImageIO/PDFKit paths
/// rather than mocks.
struct Fixtures: ~Copyable {
    let directory: URL

    init() throws {
        directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("ToolboxTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true
        )
    }

    deinit {
        try? FileManager.default.removeItem(at: directory)
    }

    /// A gradient image — compresses realistically, unlike a flat colour.
    func image(
        named name: String,
        width: Int,
        height: Int,
        format: ImageFormat = .png
    ) throws -> URL {
        let context = CGContext(
            data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!

        for y in stride(from: 0, to: height, by: 4) {
            for x in stride(from: 0, to: width, by: 4) {
                context.setFillColor(
                    red: Double(x) / Double(width),
                    green: Double(y) / Double(height),
                    blue: Double((x ^ y) % 256) / 255.0,
                    alpha: 1
                )
                context.fill(CGRect(x: x, y: y, width: 4, height: 4))
            }
        }

        let url = directory.appendingPathComponent(name)
            .appendingPathExtension(format.fileExtension)
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, format.utType.identifier as CFString, 1, nil
        )!
        CGImageDestinationAddImage(destination, context.makeImage()!, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.encodeFailed(format.displayName)
        }
        return url
    }

    /// A PDF containing real selectable text, optionally encrypted.
    ///
    /// The text matters: it's how we detect a "decryption" that silently
    /// rasterises the document and destroys searchability.
    func pdf(
        named name: String,
        password: String? = nil,
        text: String = "Toolbox test document",
        pages: Int = 1
    ) throws -> URL {
        // Build the source with CoreText so the text is genuinely extractable.
        let plain = directory.appendingPathComponent("\(name)-source")
            .appendingPathExtension("pdf")
        var mediaBox = CGRect(x: 0, y: 0, width: 400, height: 200)
        guard let context = CGContext(plain as CFURL, mediaBox: &mediaBox, nil) else {
            throw ToolboxError.writeFailed(plain)
        }
        for page in 1...max(1, pages) {
            context.beginPage(mediaBox: &mediaBox)
            let string = NSAttributedString(
                string: pages > 1 ? "\(text) \(page)" : text,
                attributes: [.font: CTFontCreateWithName("Helvetica" as CFString, 24, nil)]
            )
            context.textPosition = CGPoint(x: 20, y: 100)
            CTLineDraw(CTLineCreateWithAttributedString(string), context)
            context.endPage()
        }
        context.closePDF()

        guard let document = PDFDocument(url: plain) else {
            throw ToolboxError.notAPDF(plain)
        }

        let url = directory.appendingPathComponent(name).appendingPathExtension("pdf")
        var options: [PDFDocumentWriteOption: Any] = [:]
        if let password {
            // Both keys are required: PDFKit only encrypts when an owner
            // password is present, and the user password is what gates opening.
            options[.userPasswordOption] = password
            options[.ownerPasswordOption] = password
        }

        guard document.write(to: url, withOptions: options) else {
            throw ToolboxError.writeFailed(url)
        }
        return url
    }

    static func pixelSize(of url: URL) -> CGSize? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let w = (props[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue,
              let h = (props[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue
        else { return nil }
        return CGSize(width: w, height: h)
    }

    static func format(of url: URL) -> String? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceGetType(source) as String?
    }

    /// A real animated GIF: one gradient frame per entry in `delays`, each
    /// visibly different from the last so a dropped or duplicated frame shows up.
    ///
    /// Written with ImageIO's own GIF keys rather than through `ToolboxKit`, so a
    /// round-trip test can't pass by agreeing with a bug in the code it checks.
    func animatedGIF(
        named name: String,
        width: Int = 64,
        height: Int = 48,
        delays: [Double] = [0.1, 0.25, 0.4],
        loopCount: Int = 0
    ) throws -> URL {
        let url = directory.appendingPathComponent(name).appendingPathExtension("gif")
        guard let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.gif.identifier as CFString, delays.count, nil
        ) else {
            throw ToolboxError.writeFailed(url)
        }

        CGImageDestinationSetProperties(destination, [
            kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFLoopCount: loopCount],
        ] as CFDictionary)

        for (index, delay) in delays.enumerated() {
            let context = CGContext(
                data: nil, width: width, height: height,
                bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            )!
            for y in stride(from: 0, to: height, by: 4) {
                for x in stride(from: 0, to: width, by: 4) {
                    context.setFillColor(
                        red: Double((x + index * 17) % width) / Double(width),
                        green: Double(y) / Double(height),
                        blue: Double((x ^ y ^ (index * 40)) % 256) / 255.0,
                        alpha: 1
                    )
                    context.fill(CGRect(x: x, y: y, width: 4, height: 4))
                }
            }

            CGImageDestinationAddImage(destination, context.makeImage()!, [
                kCGImagePropertyGIFDictionary: [
                    kCGImagePropertyGIFDelayTime: delay,
                    kCGImagePropertyGIFUnclampedDelayTime: delay,
                ],
            ] as CFDictionary)
        }

        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.encodeFailed("GIF")
        }
        return url
    }

    /// Four flat quadrants, named in the order they are *displayed*: top-left,
    /// top-right, bottom-left, bottom-right.
    ///
    /// Flat colours mean a single pixel says where a crop landed and which way a
    /// rotation went, which is what the operation-order tests read back.
    func quadrantImage(
        named name: String,
        width: Int,
        height: Int,
        format: ImageFormat = .png
    ) throws -> URL {
        let context = CGContext(
            data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!

        // CGContext puts the origin at the bottom left; flipping first means the
        // rects below can be written the way the image is displayed.
        context.translateBy(x: 0, y: Double(height))
        context.scaleBy(x: 1, y: -1)

        let halfWidth = Double(width) / 2
        let halfHeight = Double(height) / 2
        let quadrants: [(CGRect, (Double, Double, Double))] = [
            (CGRect(x: 0, y: 0, width: halfWidth, height: halfHeight), (1, 0, 0)),
            (CGRect(x: halfWidth, y: 0, width: halfWidth, height: halfHeight), (0, 1, 0)),
            (CGRect(x: 0, y: halfHeight, width: halfWidth, height: halfHeight), (0, 0, 1)),
            (CGRect(x: halfWidth, y: halfHeight, width: halfWidth, height: halfHeight), (1, 1, 0)),
        ]
        for (rect, colour) in quadrants {
            context.setFillColor(red: colour.0, green: colour.1, blue: colour.2, alpha: 1)
            context.fill(rect)
        }

        let url = directory.appendingPathComponent(name)
            .appendingPathExtension(format.fileExtension)
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, format.utType.identifier as CFString, 1, nil
        )!
        CGImageDestinationAddImage(destination, context.makeImage()!, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.encodeFailed(format.displayName)
        }
        return url
    }

    /// A Display P3 image, tagged with that profile.
    ///
    /// The fill is P3's own fully saturated red, which sits outside the sRGB
    /// gamut, so a pipeline that quietly flattens to sRGB changes the file
    /// rather than merely relabelling it.
    func wideGamutImage(named name: String, width: Int = 120, height: Int = 90) throws -> URL {
        let p3 = CGColorSpace(name: CGColorSpace.displayP3)!
        let context = CGContext(
            data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: 0, space: p3,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        context.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        context.setFillColor(red: 0, green: 1, blue: 0, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width / 2, height: height / 2))

        // PNG so the profile survives losslessly and no quality dial is involved.
        let url = directory.appendingPathComponent(name).appendingPathExtension("png")
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.png.identifier as CFString, 1, nil
        )!
        CGImageDestinationAddImage(destination, context.makeImage()!, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.encodeFailed("PNG")
        }
        return url
    }

    /// A JPEG carrying an EXIF orientation tag, deliberately not square so that
    /// applying the orientation is visible in the pixel dimensions alone.
    ///
    /// Orientation 6 means "rotate a quarter turn clockwise to display", so a
    /// 40×20 file like this one is 20×40 as the user sees it.
    func orientedJPEG(
        named name: String,
        orientation: Int = 6,
        width: Int = 40,
        height: Int = 20
    ) throws -> URL {
        let context = CGContext(
            data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        context.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width / 2, height: height))
        context.setFillColor(red: 0, green: 0, blue: 1, alpha: 1)
        context.fill(CGRect(x: width / 2, y: 0, width: width / 2, height: height))

        let url = directory.appendingPathComponent(name).appendingPathExtension("jpg")
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.jpeg.identifier as CFString, 1, nil
        )!
        CGImageDestinationAddImage(destination, context.makeImage()!, [
            kCGImagePropertyOrientation: orientation,
        ] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.encodeFailed("JPEG")
        }
        return url
    }

    /// A non-square PNG split into three vertical bands — red, green, blue —
    /// each a third of the width.
    ///
    /// The bands make crop-versus-stretch unmistakable: a centred square crop
    /// of a 3:1 source keeps only the green middle, while a stretch squeezes
    /// all three bands in.
    func threeToneImage(
        named name: String,
        width: Int = 60,
        height: Int = 20
    ) throws -> URL {
        let context = CGContext(
            data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        let third = Double(width) / 3
        let bands: [(CGRect, (Double, Double, Double))] = [
            (CGRect(x: 0, y: 0, width: third, height: Double(height)), (1, 0, 0)),
            (CGRect(x: third, y: 0, width: third, height: Double(height)), (0, 1, 0)),
            (CGRect(x: third * 2, y: 0, width: third, height: Double(height)), (0, 0, 1)),
        ]
        for (rect, colour) in bands {
            context.setFillColor(red: colour.0, green: colour.1, blue: colour.2, alpha: 1)
            context.fill(rect)
        }

        let url = directory.appendingPathComponent(name).appendingPathExtension("png")
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.png.identifier as CFString, 1, nil
        )!
        CGImageDestinationAddImage(destination, context.makeImage()!, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.encodeFailed("PNG")
        }
        return url
    }

    /// A square PNG with a red square in the middle and transparency everywhere
    /// else, so alpha survival at every output size is checkable by reading a
    /// single pixel.
    func transparentImage(named name: String, side: Int = 64) throws -> URL {
        let context = CGContext(
            data: nil, width: side, height: side,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        let quarter = Double(side) / 4
        context.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
        context.fill(CGRect(
            x: quarter, y: quarter,
            width: quarter * 2, height: quarter * 2
        ))

        let url = directory.appendingPathComponent(name).appendingPathExtension("png")
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.png.identifier as CFString, 1, nil
        )!
        CGImageDestinationAddImage(destination, context.makeImage()!, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.encodeFailed("PNG")
        }
        return url
    }

    /// A PDF whose only page content is a raster image — no text operators at
    /// all, so text extraction finds nothing. This is what a scan looks like.
    func imageOnlyPDF(named name: String, width: Int = 200, height: Int = 100) throws -> URL {
        let mediaBox = CGRect(x: 0, y: 0, width: width, height: height)
        var box = mediaBox
        let url = directory.appendingPathComponent(name).appendingPathExtension("pdf")
        guard let context = CGContext(url as CFURL, mediaBox: &box, nil) else {
            throw ToolboxError.writeFailed(url)
        }
        context.beginPage(mediaBox: &box)

        let imageContext = CGContext(
            data: nil, width: width / 2, height: height / 2,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        imageContext.setFillColor(red: 0.2, green: 0.4, blue: 0.9, alpha: 1)
        imageContext.fill(CGRect(x: 0, y: 0, width: width / 2, height: height / 2))
        let image = imageContext.makeImage()!

        // Flip so the image draws upright, same trap the real tools face.
        context.saveGState()
        context.translateBy(x: 0, y: CGFloat(height))
        context.scaleBy(x: 1, y: -1)
        context.draw(image, in: mediaBox)
        context.restoreGState()

        context.endPage()
        context.closePDF()
        return url
    }

    /// A PDF with one large title line and one small body line, so heading
    /// inference from relative font sizes has something honest to find.
    func pdfWithTitleAndBody(
        named name: String,
        title: String,
        body: String
    ) throws -> URL {
        let mediaBox = CGRect(x: 0, y: 0, width: 400, height: 200)
        var box = mediaBox
        let url = directory.appendingPathComponent(name).appendingPathExtension("pdf")
        guard let context = CGContext(url as CFURL, mediaBox: &box, nil) else {
            throw ToolboxError.writeFailed(url)
        }
        context.beginPage(mediaBox: &box)

        func draw(_ string: String, size: CGFloat, y: CGFloat) {
            let attributed = NSAttributedString(
                string: string,
                attributes: [.font: CTFontCreateWithName("Helvetica" as CFString, size, nil)]
            )
            context.textPosition = CGPoint(x: 20, y: y)
            CTLineDraw(CTLineCreateWithAttributedString(attributed), context)
        }
        draw(title, size: 36, y: 120)
        draw(body, size: 12, y: 60)

        context.endPage()
        context.closePDF()
        return url
    }
}
