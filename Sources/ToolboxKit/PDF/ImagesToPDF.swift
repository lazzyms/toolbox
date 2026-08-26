import Foundation
import CoreGraphics
import ImageIO

public struct ImagesToPDFOptions: Sendable {
    public enum PageSize: Sendable {
        /// One page per image, sized to the image's own pixels.
        case fitToImage
        case a4
        case usLetter
    }

    public enum Orientation: Sendable {
        case portrait
        case landscape
    }

    public var pageSize: PageSize
    public var orientation: Orientation
    /// Fixed-size modes only: points kept clear around the image.
    public var margin: Double

    public init(
        pageSize: PageSize = .fitToImage,
        orientation: Orientation = .portrait,
        margin: Double = 36
    ) {
        self.pageSize = pageSize
        self.orientation = orientation
        self.margin = margin
    }
}

public enum ImagesToPDF {
    public static func build(
        _ inputs: [URL],
        options: ImagesToPDFOptions,
        to location: OutputLocation
    ) throws -> URL {
        guard !inputs.isEmpty else {
            throw ToolboxError.emptySelection
        }

        // Decode up front so a bad file fails before any output is written,
        // and so the page boxes can be computed for fit-to-image mode.
        var images: [(url: URL, cgImage: CGImage)] = []
        for url in inputs {
            guard let image = uprightImage(at: url) else {
                throw ToolboxError.decodeFailed(url)
            }
            images.append((url, image))
        }
        guard let first = images.first else {
            throw ToolboxError.emptySelection
        }

        let output = OutputNaming.destination(
            for: first.url, in: location, suffix: "", extension: "pdf"
        )

        var mediaBox = try pageBox(for: first.cgImage, options: options)
        guard let context = CGContext(output as CFURL, mediaBox: &mediaBox, nil) else {
            throw ToolboxError.writeFailed(output)
        }

        for entry in images {
            var box = try pageBox(for: entry.cgImage, options: options)
            context.beginPage(mediaBox: &box)
            let rect = fittedRect(
                imageSize: CGSize(width: entry.cgImage.width, height: entry.cgImage.height),
                inPage: box,
                margin: options.margin
            )
            context.draw(entry.cgImage, in: rect)
            context.endPage()
        }
        context.closePDF()
        return output
    }

    /// The rectangle an image occupies on `page`: centred, aspect preserved,
    /// never upscaled past its natural pixel size (fixed-page modes).
    public static func fittedRect(imageSize: CGSize, inPage page: CGRect, margin: Double) -> CGRect {
        let availW = max(0, page.width - CGFloat(margin) * 2)
        let availH = max(0, page.height - CGFloat(margin) * 2)
        guard imageSize.width > 0, imageSize.height > 0 else {
            return CGRect(x: page.midX, y: page.midY, width: 0, height: 0)
        }
        if page.width == imageSize.width && page.height == imageSize.height {
            return page
        }
        let scale = min(availW / imageSize.width, availH / imageSize.height, 1)
        let width = imageSize.width * scale
        let height = imageSize.height * scale
        return CGRect(
            x: page.midX - width / 2,
            y: page.midY - height / 2,
            width: width,
            height: height
        )
    }

    private static func pageBox(for image: CGImage, options: ImagesToPDFOptions) throws -> CGRect {
        let size = CGSize(width: image.width, height: image.height)
        switch options.pageSize {
        case .fitToImage:
            return CGRect(origin: .zero, size: size)
        case .a4:
            return fixedPage(width: 595, height: 842, imageSize: size, options: options)
        case .usLetter:
            return fixedPage(width: 612, height: 792, imageSize: size, options: options)
        }
    }

    private static func fixedPage(width: CGFloat, height: CGFloat, imageSize: CGSize, options: ImagesToPDFOptions) -> CGRect {
        let box: CGRect
        switch options.orientation {
        case .portrait:
            box = CGRect(x: 0, y: 0, width: width, height: height)
        case .landscape:
            box = CGRect(x: 0, y: 0, width: height, height: width)
        }
        _ = fittedRect(imageSize: imageSize, inPage: box, margin: options.margin)
        return box
    }

    /// Decodes an image with any EXIF orientation already applied — a PDF page
    /// has no orientation tag, so the rotation must land in the pixels.
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
