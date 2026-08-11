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
}
