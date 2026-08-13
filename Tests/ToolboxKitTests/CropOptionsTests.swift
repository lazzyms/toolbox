import CoreGraphics
import Foundation
import ImageIO
import Testing
@testable import ToolboxKit

/// The crop tool is a thin layer over the pipeline's `CropSpec`, which already
/// applies EXIF orientation before measuring and clamps out-of-bounds rects.
/// These pin the tool's own promises on top of that: an anchored aspect crop,
/// a pixel rect that never resamples, and a fit check that turns a mismatched
/// numeric crop into a named per-file failure instead of a silent partial crop.
@Suite("Crop")
struct CropOptionsTests {
    let processor = ImageProcessor()

    // MARK: - The spec

    @Test("a zero-area or off-canvas rect isn't a crop at all")
    func invalidRectsAreNotCrops() {
        #expect(CropOptions.aspect(width: 1, height: 1, anchor: .center).isValid)
        #expect(!CropOptions.aspect(width: 0, height: 1, anchor: .center).isValid)
        #expect(CropOptions.rect(x: 0, y: 0, width: 10, height: 10).isValid)
        #expect(!CropOptions.rect(x: 0, y: 0, width: 0, height: 10).isValid)
        #expect(!CropOptions.rect(x: -5, y: 0, width: 10, height: 10).isValid)
    }

    @Test("fit is per-image: a rect that overflows is refused with a reason")
    func numericCropFitIsChecked() {
        let fits = CropOptions.rect(x: 0, y: 0, width: 100, height: 100)
        #expect(fits.validationError(for: CGSize(width: 80, height: 40)) != nil)
        #expect(fits.validationError(for: CGSize(width: 200, height: 100)) == nil)

        // Hanging over just one edge is still a refusal, not a partial crop.
        #expect(CropOptions.rect(x: 70, y: 0, width: 40, height: 40)
            .validationError(for: CGSize(width: 80, height: 40)) != nil)
        // A rect starting off-canvas is refused too.
        #expect(CropOptions.rect(x: -5, y: 0, width: 10, height: 10)
            .validationError(for: CGSize(width: 80, height: 40)) != nil)

        // Aspect crops are sized to the image, so they can never fail a fit.
        #expect(CropOptions.aspect(width: 1, height: 1, anchor: .center)
            .validationError(for: CGSize(width: 80, height: 40)) == nil)
    }

    @Test("a crop that keeps everything is not a change at all")
    func coveringCropIsANoOp() {
        #expect(!CropOptions.aspect(width: 2, height: 1, anchor: .center)
            .isActive(for: CGSize(width: 200, height: 100)))
        #expect(CropOptions.aspect(width: 1, height: 1, anchor: .center)
            .isActive(for: CGSize(width: 200, height: 100)))
        #expect(!CropOptions.rect(x: 0, y: 0, width: 200, height: 100)
            .isActive(for: CGSize(width: 200, height: 100)))
    }

    @Test("the measured size applies the EXIF orientation a photo is tagged with")
    func pixelSizeIsAsDisplayed() throws {
        let fixtures = try Fixtures()
        // 40×20 stored, tagged for a quarter turn: 20×40 as shown.
        let input = try fixtures.orientedJPEG(named: "sideways")
        #expect(CropOptions.pixelSize(of: input) == CGSize(width: 20, height: 40))
    }

    // MARK: - Through the pipeline

    @Test("an aspect crop is anchored where it was asked to be")
    func aspectCropAnchors() throws {
        let fixtures = try Fixtures()
        // 200×100, so a square crop is 100×100 and can sit in three places.
        let input = try fixtures.quadrantImage(named: "anchors", width: 200, height: 100)

        let centred = try processor.run(input, options: .init(
            operations: [CropOptions.aspect(width: 1, height: 1, anchor: .center).operation],
            suffix: "-centre", location: .directory(fixtures.directory)
        ))
        #expect(Fixtures.pixelSize(of: centred.output) == CGSize(width: 100, height: 100))
        // Straddles the middle: red on the top left, yellow on the bottom right.
        #expect(Self.colour(centred.output, x: 10, y: 10) == .red)
        #expect(Self.colour(centred.output, x: 90, y: 90) == .yellow)

        let right = try processor.run(input, options: .init(
            operations: [CropOptions.aspect(width: 1, height: 1, anchor: .topRight).operation],
            suffix: "-right", location: .directory(fixtures.directory)
        ))
        // Hard against the top-right corner: only the green and yellow columns.
        #expect(Self.colour(right.output, x: 10, y: 10) == .green)
        #expect(Self.colour(right.output, x: 90, y: 90) == .yellow)
    }

    @Test("a pixel crop relabels the source region without resampling")
    func pixelCropIsByteIdentical() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.quadrantImage(named: "quads", width: 80, height: 40)
        let rect = CGRect(x: 20, y: 10, width: 40, height: 20)

        let result = try processor.run(input, options: .init(
            operations: [CropOptions.rect(x: 20, y: 10, width: 40, height: 20).operation],
            suffix: "-cropped", location: .directory(fixtures.directory)
        ))
        #expect(Fixtures.pixelSize(of: result.output) == CGSize(width: 40, height: 20))

        // The output must be bit-for-bit the source region, drawn through the
        // same normalisation so only the crop itself is under test.
        let sourceRegion = try #require(Self.decoded(input)).cropping(to: rect)
        #expect(Self.pixels(of: try #require(Self.decoded(result.output)))
            == Self.pixels(of: try #require(sourceRegion)))
    }

    @Test("an oriented photo crops from the region the user sees")
    func orientedPhotoCropsFromDisplayedRegion() throws {
        let fixtures = try Fixtures()
        // 40×20 stored, tagged for a quarter turn clockwise: 20×40 displayed,
        // red over blue once upright.
        let input = try fixtures.orientedJPEG(named: "upright")
        let crop = CropOptions.rect(x: 0, y: 0, width: 20, height: 30)

        // The 30-pixel-tall rect only fits the 20×40 upright image, so this
        // also pins that the fit check measures the image as displayed.
        let size = try #require(CropOptions.pixelSize(of: input))
        #expect(crop.validationError(for: size) == nil)

        let result = try processor.run(input, options: .init(
            operations: [crop.operation],
            suffix: "-cropped", location: .directory(fixtures.directory)
        ))
        #expect(Fixtures.pixelSize(of: result.output) == CGSize(width: 20, height: 30))
        // Upright, the stored left half is the top half: red above, blue below.
        #expect(Self.colour(result.output, x: 10, y: 5) == .red)
        #expect(Self.colour(result.output, x: 10, y: 25) == .blue)
    }

    // MARK: - Reading files back

    private enum Colour: Equatable { case red, green, blue, yellow, other }

    private static func decoded(_ url: URL) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(source, 0, nil)
    }

    /// Canonical sRGB RGBA bytes, top row first, so two images are comparable.
    private static func pixels(of image: CGImage) -> Data {
        let width = image.width
        let height = image.height
        var bytes = [UInt8](repeating: 0, count: width * height * 4)
        bytes.withUnsafeMutableBytes { raw in
            guard let context = CGContext(
                data: raw.baseAddress, width: width, height: height,
                bitsPerComponent: 8, bytesPerRow: width * 4,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else { return }
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        }
        return Data(bytes)
    }

    /// The colour at one pixel, in top-left-origin coordinates.
    private static func colour(_ url: URL, x: Int, y: Int) -> Colour {
        guard let image = decoded(url), x < image.width, y < image.height else { return .other }
        let data = pixels(of: image)
        let offset = (y * image.width + x) * 4
        switch (data[offset] > 170, data[offset + 1] > 170, data[offset + 2] > 170) {
        case (true, false, false): return .red
        case (false, true, false): return .green
        case (false, false, true): return .blue
        case (true, true, false): return .yellow
        default: return .other
        }
    }
}
