import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import ToolboxKit

/// The two GIF tools: stills → animation, and animation → frames. Both are
/// wrappers over `ImageFrameSequence`, so the frame/delay/loop machinery is
/// already covered by `AnimatedFrameTests`; these check the tool-shaped API —
/// the drop of stills, the raise of a sub-floor delay, and the refusal to
/// pretend a single-frame file is an animation.
@Suite("GIF builder")
struct GIFBuilderTests {

    // MARK: - Create

    @Test("create makes an animated GIF from stills")
    func createMakesAnAnimatedGIFFromStills() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let inputs = try (1...3).map { try fixtures.image(named: "frame\($0)", width: 32, height: 24) }
        let originalBytes = inputs.map { OutputNaming.fileSize(of: $0) }

        let result = try GIFBuilder.createGIF(from: inputs, options: .init(
            frameDelay: 0.1, loopCount: 3, maxDimension: nil,
            location: .directory(fixtures.directory)
        ))

        #expect(result.output.pathExtension == "gif")
        #expect(Fixtures.format(of: result.output) == UTType.gif.identifier)
        #expect(result.frameCount == 3)
        #expect(abs(result.framesPerSecond - 10) < 0.01)
        #expect(result.output.lastPathComponent == "frame1-animated.gif")
        #expect(Self.frameCount(of: result.output) == 3)
        #expect(Self.loopCount(of: result.output) == 3)
        #expect(Self.delays(of: result.output).allSatisfy { abs($0 - 0.1) < 0.01 })

        // The stills are never modified.
        for (index, input) in inputs.enumerated() {
            #expect(OutputNaming.fileSize(of: input) == originalBytes[index])
        }
    }

    @Test("frame order follows the drop order")
    func frameOrderFollowsTheDropOrder() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let inputs = [
            try Self.solidImage(named: "blue", in: fixtures.directory, red: 0, green: 0, blue: 1),
            try Self.solidImage(named: "green", in: fixtures.directory, red: 0, green: 1, blue: 0),
            try Self.solidImage(named: "red", in: fixtures.directory, red: 1, green: 0, blue: 0),
        ]

        let result = try GIFBuilder.createGIF(from: inputs, options: .init(
            frameDelay: 0.1, maxDimension: nil, location: .directory(fixtures.directory)
        ))

        let expected: [RGB] = [.init(0, 0, 255), .init(0, 255, 0), .init(255, 0, 0)]
        for index in 0..<3 {
            let colour = try #require(Self.pixelColour(of: result.output, atFrame: index, x: 0, y: 0))
            #expect(colour == expected[index])
        }
    }

    @Test("max dimension caps every frame")
    func maxDimensionCapsEveryFrame() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let inputs = try (1...2).map { try fixtures.image(named: "big\($0)", width: 200, height: 100) }

        let result = try GIFBuilder.createGIF(from: inputs, options: .init(
            frameDelay: 0.1, maxDimension: 50, location: .directory(fixtures.directory)
        ))

        for index in 0..<2 {
            let size = try #require(Self.frameSize(of: result.output, at: index))
            #expect(size == CGSize(width: 50, height: 25))
        }
    }

    @Test("sub-floor delays are raised to the 0.02s floor")
    func subFloorDelaysAreRaised() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let inputs = try (1...2).map { try fixtures.image(named: "fast\($0)", width: 16, height: 16) }

        let result = try GIFBuilder.createGIF(from: inputs, options: .init(
            frameDelay: 0.01, maxDimension: nil, location: .directory(fixtures.directory)
        ))

        #expect(abs(result.framesPerSecond - 50) < 0.01)
        #expect(Self.delays(of: result.output).allSatisfy { abs($0 - 0.02) < 0.001 })
    }

    @Test("loop count 0 means forever")
    func loopCountZeroMeansForever() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let inputs = try (1...2).map { try fixtures.image(named: "loop\($0)", width: 16, height: 16) }

        let result = try GIFBuilder.createGIF(from: inputs, options: .init(
            frameDelay: 0.1, maxDimension: nil, location: .directory(fixtures.directory)
        ))

        #expect(Self.loopCount(of: result.output) == 0)
    }

    @Test("empty input is refused with a clear message")
    func emptyInputIsRefused() throws {
        #expect(throws: ToolboxError.invalidGIFOptions("Add at least one image to animate.")) {
            _ = try GIFBuilder.createGIF(from: [], options: .init())
        }
    }

    @Test("a file that isn't an image is refused")
    func nonImageInputIsRefused() throws {
        let fixtures = try Fixtures()
        let pdf = try fixtures.pdf(named: "doc")

        #expect(throws: ToolboxError.unsupportedInput("pdf")) {
            _ = try GIFBuilder.createGIF(from: [pdf], options: .init())
        }
    }

    // MARK: - Extract

    @Test("extract writes every frame as its own PNG")
    func extractWritesEveryFrameAsItsOwnPNG() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let input = try fixtures.animatedGIF(
            named: "clip", width: 64, height: 48,
            delays: [0.1, 0.25, 0.4], loopCount: 3
        )
        let originalBytes = OutputNaming.fileSize(of: input)

        let extraction = try GIFBuilder.extractFrames(from: input, options: .init(
            location: .directory(fixtures.directory)
        ))

        #expect(extraction.outputs.count == 3)
        #expect(extraction.loopCount == 3)
        #expect(extraction.delays.count == 3)
        for (index, delay) in extraction.delays.enumerated() {
            #expect(abs(delay - [0.1, 0.25, 0.4][index]) < 0.01)
        }
        #expect(abs(extraction.totalDuration - 0.75) < 0.01)

        for (index, output) in extraction.outputs.enumerated() {
            #expect(output.pathExtension == "png")
            #expect(output.lastPathComponent == "clip-frame-\(index + 1).png")
            #expect(Fixtures.pixelSize(of: output) == CGSize(width: 64, height: 48))
            #expect(OutputNaming.fileSize(of: output) > 0)
        }

        // The animation is never modified.
        #expect(OutputNaming.fileSize(of: input) == originalBytes)
    }

    @Test("extracted frames are distinct — none dropped or duplicated")
    func extractedFramesAreDistinct() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let input = try fixtures.animatedGIF(named: "walk", delays: [0.1, 0.25, 0.4])
        let extraction = try GIFBuilder.extractFrames(from: input, options: .init(
            location: .directory(fixtures.directory)
        ))

        // The fixture draws each frame with a different red component along the
        // top edge, so a sample from one corner is enough to tell them apart.
        let reds = try extraction.outputs.map { url in
            try #require(Self.pixelColour(of: url, atFrame: 0, x: 0, y: 0)).r
        }
        #expect(Set(reds).count == 3)
    }

    @Test("a single-frame GIF is refused as not animated")
    func singleFrameGIFIsRefused() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.animatedGIF(named: "still", delays: [0.1])
        #expect(Self.frameCount(of: input) == 1)
        let before = try Self.contents(of: fixtures.directory)

        #expect(throws: ToolboxError.notAnimated(input)) {
            _ = try GIFBuilder.extractFrames(from: input, options: .init(
                location: .directory(fixtures.directory)
            ))
        }

        // A refusal writes nothing at all.
        #expect(try Self.contents(of: fixtures.directory) == before)
    }

    @Test("a still image is refused as not animated")
    func stillImageIsRefused() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "photo", width: 40, height: 30)

        #expect(throws: ToolboxError.notAnimated(input)) {
            _ = try GIFBuilder.extractFrames(from: input, options: .init(
                location: .directory(fixtures.directory)
            ))
        }
    }

    // MARK: - Round trip

    @Test("extract of a created GIF returns the same frames in the same order")
    func extractRoundTripsCreatedGIFFrames() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let inputs = [
            try Self.solidImage(named: "r", in: fixtures.directory, red: 1, green: 0, blue: 0),
            try Self.solidImage(named: "g", in: fixtures.directory, red: 0, green: 1, blue: 0),
            try Self.solidImage(named: "b", in: fixtures.directory, red: 0, green: 0, blue: 1),
        ]
        let created = try GIFBuilder.createGIF(from: inputs, options: .init(
            frameDelay: 0.1, maxDimension: nil, location: .directory(fixtures.directory)
        ))

        let extraction = try GIFBuilder.extractFrames(from: created.output, options: .init(
            location: .directory(fixtures.directory)
        ))

        #expect(extraction.outputs.count == 3)
        #expect(extraction.outputs.allSatisfy { $0.pathExtension == "png" })
        let colours = try extraction.outputs.map { url in
            try #require(Self.pixelColour(of: url, atFrame: 0, x: 0, y: 0))
        }
        #expect(colours == [.init(255, 0, 0), .init(0, 255, 0), .init(0, 0, 255)])
    }

    // MARK: - Helpers

    /// A solid-colour PNG, so a single sampled pixel identifies a frame.
    private static func solidImage(
        named name: String,
        in directory: URL,
        red: Double, green: Double, blue: Double,
        side: Int = 8
    ) throws -> URL {
        let context = CGContext(
            data: nil, width: side, height: side,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        context.setFillColor(red: red, green: green, blue: blue, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: side, height: side))

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

    /// The RGB triple of a single pixel, rendered at native size so any
    /// coordinate in the frame can be sampled.
    private struct RGB: Equatable {
        let r, g, b: UInt8
        init(_ r: UInt8, _ g: UInt8, _ b: UInt8) { self.r = r; self.g = g; self.b = b }
    }

    private static func pixelColour(
        of url: URL, atFrame index: Int, x: Int, y: Int
    ) -> RGB? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let image = CGImageSourceCreateImageAtIndex(source, index, nil)
        else { return nil }
        var data = [UInt8](repeating: 0, count: image.width * image.height * 4)
        let context = CGContext(
            data: &data, width: image.width, height: image.height,
            bitsPerComponent: 8, bytesPerRow: image.width * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        let offset = (y * image.width + x) * 4
        return RGB(data[offset], data[offset + 1], data[offset + 2])
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
            if let unclamped = (timing?[kCGImagePropertyGIFUnclampedDelayTime] as? NSNumber)?
                .doubleValue {
                return unclamped
            }
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
