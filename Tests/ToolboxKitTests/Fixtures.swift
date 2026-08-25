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

    /// A hand-assembled PDF for pinning XObject shapes Quartz's own writer
    /// never emits — JPXDecode streams, soft masks, one image object shared by
    /// two pages. Each triple is an object: the head is dictionary text with
    /// `__LENGTH__` standing wherever the stream's byte count belongs.
    ///
    /// The xref is computed from real offsets, so CoreGraphics parses it
    /// strictly rather than reconstructing and hiding mistakes.
    func rawPDF(
        named name: String,
        objects: [(id: Int, head: String, stream: Data?)]
    ) throws -> URL {
        var pdf = Data()
        pdf.append(contentsOf: "%PDF-1.4\n".utf8)
        var offsets: [Int: Int] = [:]
        for object in objects {
            offsets[object.id] = pdf.count
            pdf.append(contentsOf: "\(object.id) 0 obj\n".utf8)
            if let stream = object.stream {
                let head = object.head.replacingOccurrences(of: "__LENGTH__", with: String(stream.count))
                pdf.append(contentsOf: head.utf8)
                pdf.append(contentsOf: "\nstream\n".utf8)
                pdf.append(stream)
                pdf.append(contentsOf: "\nendstream\nendobj\n".utf8)
            } else {
                pdf.append(contentsOf: object.head.utf8)
                pdf.append(contentsOf: "\nendobj\n".utf8)
            }
        }

        let xrefOffset = pdf.count
        // Xref rows map one-to-one to ascending object numbers, so gaps
        // between sparse ids get explicit free entries.
        let highestID = objects.map(\.id).max() ?? 0
        var xref = "xref\n0 \(highestID + 1)\n0000000000 65535 f \n"
        for number in 1...highestID {
            if let offset = offsets[number] {
                xref += String(format: "%010d 00000 n \n", offset)
            } else {
                xref += "0000000000 65535 f \n"
            }
        }
        xref += "trailer\n<< /Size \(highestID + 1) /Root 1 0 R >>\nstartxref\n\(xrefOffset)\n%%EOF\n"
        pdf.append(contentsOf: xref.utf8)

        let url = directory.appendingPathComponent(name).appendingPathExtension("pdf")
        do {
            try pdf.write(to: url)
        } catch {
            throw ToolboxError.writeFailed(url)
        }
        return url
    }

    /// A blank white PDF — no text, no images — so geometric tests can read a
    /// signature's placement off an otherwise featureless canvas.
    func blankPDF(
        named name: String,
        pages: Int = 1,
        width: CGFloat = 400,
        height: CGFloat = 200
    ) throws -> URL {
        var box = CGRect(x: 0, y: 0, width: width, height: height)
        let url = directory.appendingPathComponent(name).appendingPathExtension("pdf")
        guard let context = CGContext(url as CFURL, mediaBox: &box, nil) else {
            throw ToolboxError.writeFailed(url)
        }
        for _ in 1...max(1, pages) {
            context.beginPage(mediaBox: &box)
            context.endPage()
        }
        context.closePDF()
        return url
    }

    /// A PDF whose pages show large clear text as *pixels* — text drawn into
    /// a bitmap first, then placed as an image like `imageOnlyPDF` does. This
    /// is what a scan looks like to OCR: no operators, just glyphs of light.
    func scannedTextPDF(
        named name: String,
        text: String,
        pages: Int = 1
    ) throws -> URL {
        let width = 600, height = 300
        let imageContext = CGContext(
            data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        imageContext.setFillColor(CGColor(gray: 1, alpha: 1))
        imageContext.fill(CGRect(x: 0, y: 0, width: width, height: height))
        let string = NSAttributedString(
            string: text,
            attributes: [.font: CTFontCreateWithName("Helvetica" as CFString, 72, nil)]
        )
        imageContext.textPosition = CGPoint(x: 40, y: 120)
        CTLineDraw(CTLineCreateWithAttributedString(string), imageContext)
        let image = imageContext.makeImage()!

        let mediaBox = CGRect(x: 0, y: 0, width: width, height: height)
        var box = mediaBox
        let url = directory.appendingPathComponent(name).appendingPathExtension("pdf")
        guard let context = CGContext(url as CFURL, mediaBox: &box, nil) else {
            throw ToolboxError.writeFailed(url)
        }
        for _ in 1...max(1, pages) {
            context.beginPage(mediaBox: &box)

            // Flip so the bitmap draws upright, same trap imageOnlyPDF hits.
            context.saveGState()
            context.translateBy(x: 0, y: CGFloat(height))
            context.scaleBy(x: 1, y: -1)
            context.draw(image, in: mediaBox)
            context.restoreGState()

            context.endPage()
        }
        context.closePDF()
        return url
    }

    /// A signature-shaped PNG: an opaque ink rectangle covering the *full*
    /// canvas, transparent nowhere else. Full coverage is the point — whatever
    /// draws it renders as one solid rectangle, so its ink bounding box
    /// exposes both placement and aspect ratio unambiguously.
    func signaturePNG(
        named name: String,
        width: Int = 180,
        height: Int = 60
    ) throws -> URL {
        let context = CGContext(
            data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        context.setFillColor(red: 0.05, green: 0.05, blue: 0.5, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))

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

    /// A flat-colour image so a stamped watermark shows up as a measurable
    /// darkening against a known base — single-pixel probes stay unambiguous.
    ///
    /// Always PNG: lossless, so no quality dial sits between the tool and the
    /// pixels the assertions read back.
    func solidImage(
        named name: String,
        width: Int,
        height: Int,
        colour: (red: Double, green: Double, blue: Double)
    ) throws -> URL {
        let context = CGContext(
            data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        context.setFillColor(red: colour.red, green: colour.green, blue: colour.blue, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))

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

    /// Decoded pixels in a known layout — RGBA8, sRGB, one byte per channel
    /// in R,G,B,A order — so geometric tests can probe colours without
    /// guessing the byte order ImageIO happened to hand back.
    ///
    /// 32-big is what puts R first in memory here: with a little-endian word
    /// the alpha lands in byte 0 and every channel probe silently reads the
    /// wrong channel.
    struct PixelBuffer {
        let bytes: [UInt8]
        let width: Int
        let height: Int

        func pixel(atX x: Int, y: Int) -> (r: UInt8, g: UInt8, b: UInt8, a: UInt8) {
            let offset = (y * width + x) * 4
            return (bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3])
        }

        /// Average colour over `displayRect`, whose origin is the displayed
        /// image's top-left (y grows downward), matching how fixtures name
        /// their regions.
        func meanColour(in displayRect: CGRect) -> (red: Double, green: Double, blue: Double) {
            let x0 = max(0, Int(displayRect.minX))
            let x1 = min(width - 1, Int(displayRect.maxX))
            let y0 = max(0, Int(displayRect.minY))
            let y1 = min(height - 1, Int(displayRect.maxY))
            guard x0 <= x1, y0 <= y1 else { return (0, 0, 0) }
            var red = 0.0, green = 0.0, blue = 0.0, count = 0.0
            for y in y0...y1 {
                for x in x0...x1 {
                    let p = pixel(atX: x, y: y)
                    red += Double(p.r)
                    green += Double(p.g)
                    blue += Double(p.b)
                    count += 1
                }
            }
            return (red / count, green / count, blue / count)
        }

        /// How many pixels in `displayRect` read as "ink": opaque enough to be
        /// real and every channel below `maxChannelBelow`, which black ink on
        /// any bright base satisfies.
        func inkPixelCount(in displayRect: CGRect, maxChannelBelow threshold: UInt8) -> Int {
            let x0 = max(0, Int(displayRect.minX))
            let x1 = min(width - 1, Int(displayRect.maxX))
            let y0 = max(0, Int(displayRect.minY))
            let y1 = min(height - 1, Int(displayRect.maxY))
            var count = 0
            for y in y0...max(y0, y1) {
                for x in x0...max(x0, x1) where pixel(atX: x, y: y).a > 0 {
                    let p = pixel(atX: x, y: y)
                    if max(p.r, max(p.g, p.b)) < threshold { count += 1 }
                }
            }
            return count
        }

        /// Bounding box of all ink pixels (same test as `inkPixelCount`) in
        /// display coordinates, or nil when nothing matched.
        func inkBoundingBox(maxChannelBelow threshold: UInt8) -> CGRect? {
            var minX = Int.max, maxX = Int.min, minY = Int.max, maxY = Int.min
            for y in 0..<height {
                for x in 0..<width where pixel(atX: x, y: y).a > 0 {
                    let p = pixel(atX: x, y: y)
                    if max(p.r, max(p.g, p.b)) < threshold {
                        minX = min(minX, x); maxX = max(maxX, x)
                        minY = min(minY, y); maxY = max(maxY, y)
                    }
                }
            }
            guard minX <= maxX else { return nil }
            return CGRect(x: minX, y: minY, width: maxX - minX + 1, height: maxY - minY + 1)
        }
    }

    /// Reads `url` through the same upright decode the watermarker uses, then
    /// redraws into the canonical RGBA buffer.
    static func pixels(of url: URL) throws -> PixelBuffer {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else {
            throw ToolboxError.decodeFailed(url)
        }
        let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        let declaredWidth = (props?[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue ?? 1
        let declaredHeight = (props?[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue ?? 1
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: max(declaredWidth, declaredHeight),
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary),
              let context = CGContext(
                data: nil, width: image.width, height: image.height,
                bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
                    | CGImageByteOrderInfo.order32Big.rawValue
              )
        else {
            throw ToolboxError.decodeFailed(url)
        }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))

        // Row stride can carry padding, so copy row by row into a tight buffer.
        let bytesPerRow = context.bytesPerRow
        var bytes = [UInt8](repeating: 0, count: image.width * image.height * 4)
        if let raw = context.data {
            bytes.withUnsafeMutableBytes { destination in
                let sourceBase = raw.assumingMemoryBound(to: UInt8.self)
                for row in 0..<image.height {
                    memcpy(
                        destination.baseAddress! + row * image.width * 4,
                        sourceBase + row * bytesPerRow,
                        image.width * 4
                    )
                }
            }
        }
        return PixelBuffer(bytes: bytes, width: image.width, height: image.height)
    }

    /// An image carrying the metadata a real photo leaks: camera make/model,
    /// lens, timestamp, GPS coordinates and IPTC attribution, plus an
    /// orientation tag of 6 so tests can watch what stripping does to it.
    ///
    /// Written through ImageIO's property dictionaries directly — the same
    /// route a phone camera takes — so a strip test can't pass by agreeing
    /// with a bug in ToolboxKit's own writing path.
    func taggedImage(
        named name: String,
        width: Int = 64,
        height: Int = 48,
        format: ImageFormat = .jpeg
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
        CGImageDestinationAddImage(destination, context.makeImage()!, [
            kCGImagePropertyExifDictionary: [
                kCGImagePropertyExifDateTimeOriginal: "2026:08:25 10:30:00",
                kCGImagePropertyExifLensModel: "TestLens 50mm",
                kCGImagePropertyExifPixelXDimension: width,
                kCGImagePropertyExifPixelYDimension: height,
            ],
            kCGImagePropertyGPSDictionary: [
                kCGImagePropertyGPSLatitude: 37.3349,
                kCGImagePropertyGPSLatitudeRef: "N",
                kCGImagePropertyGPSLongitude: 122.0090,
                kCGImagePropertyGPSLongitudeRef: "W",
            ],
            kCGImagePropertyIPTCDictionary: [
                kCGImagePropertyIPTCCopyrightNotice: "(c) Probe Photographer",
                kCGImagePropertyIPTCCredit: "Probe Credit",
            ],
            kCGImagePropertyTIFFDictionary: [
                kCGImagePropertyTIFFMake: "ProbeCam",
                kCGImagePropertyTIFFModel: "Probe One",
                kCGImagePropertyTIFFSoftware: "ProbeSoft",
                kCGImagePropertyTIFFOrientation: 6,
            ],
            kCGImagePropertyOrientation: 6,
        ] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.encodeFailed(format.displayName)
        }
        return url
    }


    /// A uniform grey field.
    ///
    /// A flat fill means a tone adjustment shows up as a shift of the
    /// whole-image mean rather than as movement inside local structure, which
    /// keeps brightness/exposure checks free of clipping and resampling noise.
    func flatImage(
        named name: String,
        width: Int = 64,
        height: Int = 64,
        level: Double = 0.5
    ) throws -> URL {
        let context = CGContext(
            data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        context.setFillColor(gray: level, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))

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
}
