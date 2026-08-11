import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import ToolboxKit

/// The middle of the image pipeline is an ordered operation list rather than a
/// single resize. These cover the properties that made the old single-purpose
/// code correct, because a general pipeline is exactly where they get lost:
/// operation order, the resize fast path, wide-gamut colour and EXIF
/// orientation being spent exactly once.
@Suite("Image operations")
struct ImageOperationTests {
    let processor = ImageProcessor()

    // MARK: - Choosing a route

    @Test("a lone resize picks the thumbnail route and anything else the pipeline")
    func planPicksTheCheapestRoute() {
        #expect(ImageRenderPlan(operations: []) == .passthrough)

        // Steps that cannot change a pixel are dropped, so convert and compress
        // keep the plain decode-and-re-encode route they have always had.
        #expect(ImageRenderPlan(operations: [.resize(.none)]) == .passthrough)
        #expect(ImageRenderPlan(operations: [
            .rotate(degrees: 0), .flip(horizontal: false, vertical: false),
        ]) == .passthrough)

        #expect(ImageRenderPlan(operations: [.resize(.fit(width: 400, height: 400))])
            == .thumbnail(.fit(width: 400, height: 400)))
        // A no-op alongside a resize still leaves exactly one real operation.
        #expect(ImageRenderPlan(operations: [.resize(.percent(50)), .rotate(degrees: 0)])
            == .thumbnail(.percent(50)))

        #expect(ImageRenderPlan(operations: [.rotate(degrees: 90)])
            == .pipeline([.rotate(degrees: 90)]))
        let cropThenResize: [ImageOperation] = [
            .crop(.aspect(width: 1, height: 1, anchor: .center)), .resize(.percent(50)),
        ]
        #expect(ImageRenderPlan(operations: cropThenResize) == .pipeline(cropThenResize))
    }

    @Test("a lone resize still decodes through ImageIO's thumbnail, not a redraw")
    func resizeKeepsTheThumbnailFastPath() throws {
        let fixtures = try Fixtures()
        // A gradient, so two scaling algorithms have something to disagree about.
        let input = try fixtures.image(named: "grad", width: 800, height: 600)

        let result = try processor.run(input, options: .init(
            resize: .fit(width: 400, height: 400), suffix: "-r",
            location: .directory(fixtures.directory)
        ))
        #expect(Fixtures.pixelSize(of: result.output) == CGSize(width: 400, height: 300))

        // Built here with ImageIO directly, so matching it can't mean agreeing
        // with a bug in the code under test.
        let expected = Self.pixels(of: try #require(Self.thumbnail(of: input, maxPixelSize: 400)))
        let actual = Self.pixels(of: try #require(Self.decoded(result.output)))
        #expect(actual == expected)

        // And the comparison discriminates: the pipeline route scales with
        // Lanczos, which resamples differently from ImageIO's thumbnail, so a
        // regression that dropped the fast path would produce these pixels
        // instead of the ones above. (A plain CGContext redraw is *not* a
        // discriminator — on this OS ImageIO's thumbnail is implemented by one,
        // so the two are bit-identical.)
        let viaPipeline = try ImageOperationRenderer.apply(
            [.resize(.fit(width: 400, height: 400))],
            to: try #require(Self.decoded(input)),
            orientation: .up,
            allowUpscale: false
        )
        #expect(Self.pixels(of: viaPipeline.image) != expected)
    }

    @Test("the resize convenience initialiser matches an explicit operation list")
    func convenienceInitialiserMatchesOperations() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "same", width: 300, height: 200)

        let viaConvenience = try processor.run(input, options: .init(
            resize: .percent(50), suffix: "-a", location: .directory(fixtures.directory)
        ))
        let viaOperations = try processor.run(input, options: .init(
            operations: [.resize(.percent(50))], suffix: "-b",
            location: .directory(fixtures.directory)
        ))

        #expect(Fixtures.pixelSize(of: viaConvenience.output) == CGSize(width: 150, height: 100))
        #expect(Self.pixels(of: try #require(Self.decoded(viaConvenience.output)))
            == Self.pixels(of: try #require(Self.decoded(viaOperations.output))))
    }

    // MARK: - Order

    @Test("crop then resize is not the same as resize then crop")
    func operationsComposeInTheOrderGiven() throws {
        let fixtures = try Fixtures()
        // 200×100: red | green across the top, blue | yellow underneath.
        let input = try fixtures.quadrantImage(named: "quads", width: 200, height: 100)

        // Crop the left half first, so red over blue survives into the squash.
        let cropFirst = try processor.run(input, options: .init(
            operations: [
                .crop(.rect(x: 0, y: 0, width: 100, height: 100)),
                .resize(.exact(width: 50, height: 50)),
            ],
            suffix: "-crop-resize", location: .directory(fixtures.directory)
        ))

        // Squash the whole image first, and the top-left 50×50 is all red.
        let resizeFirst = try processor.run(input, options: .init(
            operations: [
                .resize(.exact(width: 100, height: 100)),
                .crop(.rect(x: 0, y: 0, width: 50, height: 50)),
            ],
            suffix: "-resize-crop", location: .directory(fixtures.directory)
        ))

        // Identical dimensions either way, so only the content tells them apart
        // — which is the point: order is not cosmetic.
        #expect(Fixtures.pixelSize(of: cropFirst.output) == CGSize(width: 50, height: 50))
        #expect(Fixtures.pixelSize(of: resizeFirst.output) == CGSize(width: 50, height: 50))

        #expect(Self.colour(cropFirst.output, x: 25, y: 10) == .red)
        #expect(Self.colour(cropFirst.output, x: 25, y: 40) == .blue)
        #expect(Self.colour(resizeFirst.output, x: 25, y: 10) == .red)
        #expect(Self.colour(resizeFirst.output, x: 25, y: 40) == .red)
    }

    @Test("rotate turns clockwise and flip mirrors")
    func rotateAndFlipDirections() throws {
        let fixtures = try Fixtures()
        // 80×40: red | green on top, blue | yellow underneath.
        let input = try fixtures.quadrantImage(named: "dial", width: 80, height: 40)

        let rotated = try processor.run(input, options: .init(
            operations: [.rotate(degrees: 90)], suffix: "-r90",
            location: .directory(fixtures.directory)
        ))
        // A quarter turn clockwise swaps the sides and carries the top-left red
        // quadrant to the top right.
        #expect(Fixtures.pixelSize(of: rotated.output) == CGSize(width: 40, height: 80))
        #expect(Self.colour(rotated.output, x: 30, y: 10) == .red)
        #expect(Self.colour(rotated.output, x: 10, y: 10) == .blue)

        let flipped = try processor.run(input, options: .init(
            operations: [.flip(horizontal: true, vertical: false)], suffix: "-flip",
            location: .directory(fixtures.directory)
        ))
        #expect(Fixtures.pixelSize(of: flipped.output) == CGSize(width: 80, height: 40))
        // Mirroring brings the green top-right quadrant over to the left.
        #expect(Self.colour(flipped.output, x: 10, y: 10) == .green)
        #expect(Self.colour(flipped.output, x: 70, y: 10) == .red)
    }

    @Test("rotation has to be a quarter turn")
    func rotationMustBeAQuarterTurn() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "tilt", width: 60, height: 40)
        let before = try Self.contents(of: fixtures.directory)

        #expect(throws: ToolboxError.unsupportedRotation(45)) {
            try processor.run(input, options: .init(
                operations: [.rotate(degrees: 45)], location: .directory(fixtures.directory)
            ))
        }
        // A refusal writes nothing at all.
        #expect(try Self.contents(of: fixtures.directory) == before)

        #expect(ImageOperation.quarterTurns(270) == 3)
        #expect(ImageOperation.quarterTurns(-90) == 3)
        #expect(ImageOperation.quarterTurns(45) == nil)
        // Whole turns normalise away rather than costing a render.
        #expect(ImageRenderPlan(operations: [.rotate(degrees: 720)]) == .passthrough)
    }

    // MARK: - Crop geometry

    @Test("an aspect crop is anchored where it was asked to be")
    func aspectCropAnchors() throws {
        let fixtures = try Fixtures()
        // 200×100, so a square crop is 100×100 and can sit in three places.
        let input = try fixtures.quadrantImage(named: "anchors", width: 200, height: 100)

        let centred = try processor.run(input, options: .init(
            operations: [.crop(.aspect(width: 1, height: 1, anchor: .center))],
            suffix: "-centre", location: .directory(fixtures.directory)
        ))
        #expect(Fixtures.pixelSize(of: centred.output) == CGSize(width: 100, height: 100))
        // Straddles the middle: red on the top left, green on the top right.
        #expect(Self.colour(centred.output, x: 10, y: 10) == .red)
        #expect(Self.colour(centred.output, x: 90, y: 10) == .green)
        #expect(Self.colour(centred.output, x: 90, y: 90) == .yellow)

        let right = try processor.run(input, options: .init(
            operations: [.crop(.aspect(width: 1, height: 1, anchor: .topRight))],
            suffix: "-right", location: .directory(fixtures.directory)
        ))
        // Hard against the right edge, so only the green and yellow columns.
        #expect(Self.colour(right.output, x: 10, y: 10) == .green)
        #expect(Self.colour(right.output, x: 90, y: 90) == .yellow)
    }

    @Test("a crop that keeps everything is not a change at all")
    func cropCoveringTheWholeImageIsANoOp() {
        let source = CGSize(width: 200, height: 100)
        #expect(CropSpec.rect(x: 0, y: 0, width: 200, height: 100).rect(for: source) == nil)
        // Oversized rects clamp to the image, and so also keep everything.
        #expect(CropSpec.rect(x: 0, y: 0, width: 9999, height: 9999).rect(for: source) == nil)
        #expect(CropSpec.rect(x: 0, y: 0, width: 0, height: 0).rect(for: source) == nil)
        // Entirely outside the image: nothing to keep.
        #expect(CropSpec.rect(x: 500, y: 500, width: 10, height: 10).rect(for: source) == nil)

        #expect(CropSpec.rect(x: 10, y: 20, width: 50, height: 40).rect(for: source)
            == CGRect(x: 10, y: 20, width: 50, height: 40))
        // A rect hanging over the edge keeps the part that overlaps.
        #expect(CropSpec.rect(x: 180, y: 0, width: 50, height: 40).rect(for: source)
            == CGRect(x: 180, y: 0, width: 20, height: 40))
        #expect(CropSpec.aspect(width: 1, height: 1, anchor: .center).rect(for: source)
            == CGRect(x: 50, y: 0, width: 100, height: 100))
    }

    // MARK: - Colour

    @Test("a Display P3 source does not come back as sRGB")
    func wideGamutSurvives() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.wideGamutImage(named: "vivid")
        #expect(try #require(Self.colorSpaceName(of: input)).contains("P3"))

        // The pipeline route renders into a fresh bitmap, so it is the one that
        // could quietly flatten the gamut.
        let cropped = try processor.run(input, options: .init(
            operations: [.crop(.aspect(width: 1, height: 1, anchor: .center))],
            stripMetadata: true, suffix: "-cropped", location: .directory(fixtures.directory)
        ))
        #expect(Self.isP3(cropped.output))
        #expect(Self.colorSpaceName(of: cropped.output)?.contains("sRGB") == false)

        // A longer chain still ends in one render, and still in P3.
        let chained = try processor.run(input, options: .init(
            operations: [
                .crop(.rect(x: 0, y: 0, width: 100, height: 80)),
                .rotate(degrees: 180),
                .resize(.percent(50)),
            ],
            suffix: "-chained", location: .directory(fixtures.directory)
        ))
        #expect(Self.isP3(chained.output))
        #expect(Fixtures.pixelSize(of: chained.output) == CGSize(width: 50, height: 40))

        // And the resize fast path, where ImageIO preserves the profile for us.
        let resized = try processor.run(input, options: .init(
            resize: .percent(50), suffix: "-resized",
            location: .directory(fixtures.directory)
        ))
        #expect(Self.isP3(resized.output))
    }

    // MARK: - Orientation

    @Test("EXIF orientation is applied exactly once, whichever route runs")
    func orientationAppliedExactlyOnce() throws {
        let fixtures = try Fixtures()
        // 40×20 stored, tagged for a quarter turn clockwise: 20×40 displayed,
        // red over blue once upright.
        let input = try fixtures.orientedJPEG(named: "upright")
        #expect(Fixtures.pixelSize(of: input) == CGSize(width: 40, height: 20))
        #expect(Self.orientationTag(of: input) == 6)

        // Resize fast path: ImageIO bakes the rotation into the pixels, so the
        // tag has been spent and writing it again would rotate them twice.
        let resized = try processor.run(input, options: .init(
            resize: .percent(50), suffix: "-resized",
            location: .directory(fixtures.directory)
        ))
        #expect(Fixtures.pixelSize(of: resized.output) == CGSize(width: 10, height: 20))
        #expect(Self.orientationTag(of: resized.output) == nil)
        #expect(Self.tiffOrientationTag(of: resized.output) == nil)

        // Pipeline route: the same promise by a different road. The crop is 30
        // tall, which only fits the 20×40 upright image — so this also pins
        // orientation as being applied *before* the operations, not after.
        let cropped = try processor.run(input, options: .init(
            operations: [.crop(.rect(x: 0, y: 0, width: 20, height: 30))],
            suffix: "-cropped", location: .directory(fixtures.directory)
        ))
        #expect(Fixtures.pixelSize(of: cropped.output) == CGSize(width: 20, height: 30))
        #expect(Self.orientationTag(of: cropped.output) == nil)
        #expect(Self.tiffOrientationTag(of: cropped.output) == nil)
        // Upright, the stored left half is the top half: red above, blue below.
        #expect(Self.colour(cropped.output, x: 10, y: 5) == .red)
        #expect(Self.colour(cropped.output, x: 10, y: 25) == .blue)

        // With no operations nothing rotates the pixels, so the tag is still
        // what keeps the image upright and it has to survive.
        let converted = try processor.run(input, options: .init(
            format: .jpeg, suffix: "-converted", location: .directory(fixtures.directory)
        ))
        #expect(Fixtures.pixelSize(of: converted.output) == CGSize(width: 40, height: 20))
        #expect(Self.tiffOrientationTag(of: converted.output) == 6)
    }

    // MARK: - The no-inflation guard

    @Test("the no-inflation guard yields to an operation that changed the pixels")
    func guardYieldsToRealOperations() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "flat", width: 400, height: 300, format: .png)
        let originalBytes = OutputNaming.fileSize(of: input)

        // A crop is a change the user asked for, so the output stands even if
        // re-encoding costs more bytes than the original did.
        let cropped = try processor.run(input, options: .init(
            format: .png,
            operations: [.crop(.aspect(width: 1, height: 1, anchor: .center))],
            keepSmallerOriginal: true, suffix: "-cropped",
            location: .directory(fixtures.directory)
        ))
        #expect(cropped.keptOriginal == false)
        #expect(Fixtures.pixelSize(of: cropped.output) == CGSize(width: 300, height: 300))

        // A crop that resolves to the whole image changes nothing, so the guard
        // is back in force and the original bytes win.
        let unchanged = try processor.run(input, options: .init(
            format: .png,
            operations: [.crop(.rect(x: 0, y: 0, width: 9999, height: 9999))],
            keepSmallerOriginal: true, suffix: "-same",
            location: .directory(fixtures.directory)
        ))
        #expect(unchanged.newBytes <= originalBytes)
    }

    // MARK: - Animations

    @Test("every frame of an animation goes through the whole operation list")
    func multiFrameRunsTheWholePipeline() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let input = try fixtures.animatedGIF(
            named: "spin", width: 64, height: 48,
            delays: [0.1, 0.25, 0.4], loopCount: 2
        )

        // Two operations, so this is the pipeline route rather than #28's
        // thumbnail-per-frame one, and it still has to reach every frame.
        let result = try processor.run(input, options: .init(
            operations: [
                .crop(.rect(x: 0, y: 0, width: 32, height: 48)),
                .resize(.percent(50)),
            ],
            suffix: "-cropped", location: .directory(fixtures.directory)
        ))

        #expect(result.output.pathExtension == "gif")
        #expect(Self.frameCount(of: result.output) == 3)
        for index in 0..<3 {
            #expect(Self.frameSize(of: result.output, at: index) == CGSize(width: 16, height: 24))
        }
        #expect(result.pixelSize == CGSize(width: 16, height: 24))
        // Timing and looping are still the animation's own.
        #expect(Self.delays(of: result.output) == [0.1, 0.25, 0.4])
        #expect(Self.loopCount(of: result.output) == 2)
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

    /// The colour at one pixel, in top-left-origin coordinates. Thresholds are
    /// generous because the fixtures are flat fills and the point is *which*
    /// quadrant landed here, not its exact value after resampling.
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

    private static func thumbnail(of url: URL, maxPixelSize: Int) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceCreateThumbnailAtIndex(source, 0, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
            kCGImageSourceShouldCacheImmediately: true,
        ] as CFDictionary)
    }

    private static func colorSpaceName(of url: URL) -> String? {
        guard let image = decoded(url), let space = image.colorSpace else { return nil }
        if let name = space.name { return name as String }
        if space.model == .rgb { return "RGB" }
        return nil
    }

    private static func isP3(_ url: URL) -> Bool {
        guard let image = decoded(url), let space = image.colorSpace else { return false }
        let name = (space.name as String?) ?? "unknown"
        if name.contains("P3") { return true }
        let p3 = CGColorSpace(name: CGColorSpace.displayP3)
        return space == p3
    }

    private static func properties(of url: URL) -> [CFString: Any]? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
    }

    private static func orientationTag(of url: URL) -> Int? {
        (properties(of: url)?[kCGImagePropertyOrientation] as? NSNumber)?.intValue
    }

    /// ImageIO reports the orientation in the TIFF dictionary as well as at the
    /// top level, and that copy is the one that used to ride along unnoticed.
    private static func tiffOrientationTag(of url: URL) -> Int? {
        let tiff = properties(of: url)?[kCGImagePropertyTIFFDictionary] as? [CFString: Any]
        return (tiff?[kCGImagePropertyTIFFOrientation] as? NSNumber)?.intValue
    }

    private static func frameCount(of url: URL) -> Int {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return 0 }
        return CGImageSourceGetCount(source)
    }

    private static func frameSize(of url: URL, at index: Int) -> CGSize? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let image = CGImageSourceCreateImageAtIndex(source, index, nil)
        else { return nil }
        return CGSize(width: image.width, height: image.height)
    }

    private static func delays(of url: URL) -> [Double] {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return [] }
        return (0..<CGImageSourceGetCount(source)).compactMap { index in
            let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil)
                as? [CFString: Any]
            let timing = properties?[kCGImagePropertyGIFDictionary] as? [CFString: Any]
            return (timing?[kCGImagePropertyGIFDelayTime] as? NSNumber)?.doubleValue
        }
    }

    private static func loopCount(of url: URL) -> Int? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let properties = CGImageSourceCopyProperties(source, nil) as? [CFString: Any],
              let gif = properties[kCGImagePropertyGIFDictionary] as? [CFString: Any]
        else { return nil }
        return (gif[kCGImagePropertyGIFLoopCount] as? NSNumber)?.intValue
    }

    private static func contents(of directory: URL) throws -> Set<String> {
        Set(try FileManager.default.contentsOfDirectory(atPath: directory.path))
    }
}
