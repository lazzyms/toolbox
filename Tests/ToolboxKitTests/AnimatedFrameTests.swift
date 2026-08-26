import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import ToolboxKit

/// An animated GIF used to arrive back as a single still frame, reported as a
/// successful conversion. These cover the two acceptable outcomes — every frame
/// survives, or the run fails and says why — and nothing in between.
@Suite("Animated frames")
struct AnimatedFrameTests {
    let processor = ImageProcessor()

    @Test("resize keeps every frame, its delay and the loop count")
    func resizeKeepsEveryFrameAndItsTiming() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let input = try fixtures.animatedGIF(
            named: "clip", width: 64, height: 48,
            delays: [0.1, 0.25, 0.4], loopCount: 3
        )
        let originalBytes = OutputNaming.fileSize(of: input)

        let result = try processor.run(input, options: .init(
            resize: .percent(50), suffix: "-resized",
            location: .directory(fixtures.directory)
        ))

        // Still a GIF, and still animated.
        #expect(result.output.pathExtension == "gif")
        #expect(Fixtures.format(of: result.output) == UTType.gif.identifier)
        #expect(Self.frameCount(of: result.output) == 3)

        // Every frame scaled, not just the first.
        for index in 0..<3 {
            #expect(Self.frameSize(of: result.output, at: index) == CGSize(width: 32, height: 24))
        }

        #expect(Self.delays(of: result.output) == [0.1, 0.25, 0.4])
        #expect(Self.loopCount(of: result.output) == 3)
        #expect(result.pixelSize == CGSize(width: 32, height: 24))
        #expect(result.originalPixelSize == CGSize(width: 64, height: 48))

        // The original is never touched.
        #expect(Self.frameCount(of: input) == 3)
        #expect(OutputNaming.fileSize(of: input) == originalBytes)
    }

    @Test("compress keeps every frame and never grows the file")
    func compressKeepsEveryFrame() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let input = try fixtures.animatedGIF(named: "loop", delays: [0.2, 0.2, 0.2, 0.2])
        let originalBytes = OutputNaming.fileSize(of: input)

        // Lossless compress: keep the input's format, fall back to the original
        // bytes if re-encoding grew it. Either way the animation must survive.
        let result = try processor.run(input, options: .init(
            format: nil, quality: 1.0, stripMetadata: true,
            keepSmallerOriginal: true, suffix: "-compressed",
            location: .directory(fixtures.directory)
        ))

        #expect(result.output.pathExtension == "gif")
        #expect(Self.frameCount(of: result.output) == 4)
        #expect(Self.delays(of: result.output) == [0.2, 0.2, 0.2, 0.2])
        #expect(result.newBytes <= originalBytes)
    }

    @Test("exact resize redraws every frame to the requested box")
    func exactResizeAppliesToEveryFrame() throws {
        try #require(ImageFrameSequence.Container.gif.canWriteFrames)
        let fixtures = try Fixtures()
        let input = try fixtures.animatedGIF(named: "square", width: 80, height: 60)

        let result = try processor.run(input, options: .init(
            resize: .exact(width: 40, height: 40), allowUpscale: true,
            suffix: "-resized", location: .directory(fixtures.directory)
        ))

        #expect(Self.frameCount(of: result.output) == 3)
        for index in 0..<3 {
            #expect(Self.frameSize(of: result.output, at: index) == CGSize(width: 40, height: 40))
        }
    }

    @Test("converting to a still format fails instead of flattening")
    func convertingToAStillFormatFailsInsteadOfFlattening() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.animatedGIF(named: "banner", delays: [0.1, 0.1, 0.1])
        let before = try Self.contents(of: fixtures.directory)

        for format in [ImageFormat.png, .jpeg] {
            #expect(throws: ToolboxError.wouldDropFrames(
                input, frames: 3, format: format.displayName
            )) {
                try processor.run(input, options: .init(
                    format: format, location: .directory(fixtures.directory)
                ))
            }
        }

        // A refusal writes nothing at all — no half-made still image left behind.
        #expect(try Self.contents(of: fixtures.directory) == before)
        #expect(Self.frameCount(of: input) == 3)
    }

    @Test("the error names the file, the frame count and the format")
    func errorReadsClearly() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.animatedGIF(named: "walk", delays: [0.1, 0.2])
        let error = ToolboxError.wouldDropFrames(input, frames: 2, format: "PNG")

        #expect(error.errorDescription == "“walk.gif” has 2 frames, and a PNG file "
            + "can only hold the first one.")
        #expect(error.recoverySuggestion?.isEmpty == false)
    }

    @Test("a single-frame GIF converts as it always did")
    func singleFrameGIFStillConverts() throws {
        let fixtures = try Fixtures()
        // One frame is not an animation, so the refusal must not fire here.
        let input = try fixtures.animatedGIF(named: "still", delays: [0.1])
        #expect(Self.frameCount(of: input) == 1)

        let result = try processor.run(input, options: .init(
            format: .png, location: .directory(fixtures.directory)
        ))

        #expect(Fixtures.format(of: result.output) == UTType.png.identifier)
        #expect(result.pixelSize == CGSize(width: 64, height: 48))
    }

    @Test("multi-page TIFF keeps its pages")
    func multiPageTIFFKeepsItsPages() throws {
        try #require(ImageFrameSequence.Container.tiff.canWriteFrames)
        let fixtures = try Fixtures()
        let input = try Self.multiFrameFile(
            in: fixtures.directory, named: "scan", extension: "tiff",
            type: UTType.tiff.identifier, frames: 3, width: 100, height: 80
        )

        let result = try processor.run(input, options: .init(
            resize: .longestSide(50), suffix: "-resized",
            location: .directory(fixtures.directory)
        ))

        #expect(result.output.pathExtension == "tiff")
        #expect(Self.frameCount(of: result.output) == 3)
        for index in 0..<3 {
            #expect(Self.frameSize(of: result.output, at: index) == CGSize(width: 50, height: 40))
        }
    }

    @Test("animated PNG keeps its frames and delays")
    func animatedPNGKeepsItsFrames() throws {
        try #require(ImageFrameSequence.Container.apng.canWriteFrames)
        let fixtures = try Fixtures()
        let input = try Self.multiFrameFile(
            in: fixtures.directory, named: "spinner", extension: "png",
            type: UTType.png.identifier, frames: 3, width: 64, height: 48,
            dictionaryKey: kCGImagePropertyPNGDictionary,
            delayKey: kCGImagePropertyAPNGDelayTime,
            loopCountKey: kCGImagePropertyAPNGLoopCount,
            delay: 0.25
        )
        #expect(Self.frameCount(of: input) == 3)

        // PNG is the input's own format here, so this is a keep-the-format run
        // and the frames can be carried across.
        let result = try processor.run(input, options: .init(
            format: .png, resize: .percent(50), suffix: "-resized",
            location: .directory(fixtures.directory)
        ))

        #expect(Self.frameCount(of: result.output) == 3)
        let delays = Self.delays(
            of: result.output,
            dictionaryKey: kCGImagePropertyPNGDictionary,
            delayKey: kCGImagePropertyAPNGDelayTime
        )
        // APNG stores delays as a fraction, so compare with a tolerance.
        #expect(delays.count == 3)
        for delay in delays { #expect(abs(delay - 0.25) < 0.01) }
    }

    @Test("animated WebP is refused rather than written as one frame")
    func animatedWebPIsRefused() throws {
        // ImageIO has never shipped an animated WebP encoder, so a WebP that
        // this Mac can otherwise read must fail honestly instead of flattening.
        let container = try #require(
            ImageFrameSequence.Container(typeIdentifier: "org.webmproject.webp")
        )
        #expect(container.canWriteFrames == false)
        #expect(container.displayName == "WebP")

        let error = ToolboxError.cannotWriteFrames(
            URL(fileURLWithPath: "/tmp/logo.webp"), frames: 12, format: container.displayName
        )
        #expect(error.errorDescription?.contains("12 frames") == true)
        #expect(error.errorDescription?.contains("WebP") == true)
    }

    // MARK: - Reading files back

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

    private static func delays(
        of url: URL,
        dictionaryKey: CFString = kCGImagePropertyGIFDictionary,
        delayKey: CFString = kCGImagePropertyGIFDelayTime
    ) -> [Double] {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return [] }
        return (0..<CGImageSourceGetCount(source)).compactMap { index in
            let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil)
                as? [CFString: Any]
            let timing = properties?[dictionaryKey] as? [CFString: Any]
            return (timing?[delayKey] as? NSNumber)?.doubleValue
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

    /// A multi-frame file in any container ImageIO can write, built with the
    /// literal ImageIO keys so the test doesn't lean on the code under test.
    private static func multiFrameFile(
        in directory: URL,
        named name: String,
        extension ext: String,
        type: String,
        frames: Int,
        width: Int,
        height: Int,
        dictionaryKey: CFString? = nil,
        delayKey: CFString? = nil,
        loopCountKey: CFString? = nil,
        delay: Double = 0
    ) throws -> URL {
        let url = directory.appendingPathComponent(name).appendingPathExtension(ext)
        guard let destination = CGImageDestinationCreateWithURL(
            url as CFURL, type as CFString, frames, nil
        ) else {
            throw ToolboxError.writeFailed(url)
        }

        if let dictionaryKey, let loopCountKey {
            CGImageDestinationSetProperties(
                destination, [dictionaryKey: [loopCountKey: 0]] as CFDictionary
            )
        }

        for index in 0..<frames {
            let context = CGContext(
                data: nil, width: width, height: height,
                bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            )!
            for y in stride(from: 0, to: height, by: 4) {
                for x in stride(from: 0, to: width, by: 4) {
                    context.setFillColor(
                        red: Double((x + index * 13) % width) / Double(width),
                        green: Double(y) / Double(height),
                        blue: Double((x ^ y ^ (index * 30)) % 256) / 255.0,
                        alpha: 1
                    )
                    context.fill(CGRect(x: x, y: y, width: 4, height: 4))
                }
            }

            var properties: [CFString: Any] = [:]
            if let dictionaryKey, let delayKey {
                properties[dictionaryKey] = [delayKey: delay]
            }
            CGImageDestinationAddImage(
                destination, context.makeImage()!, properties as CFDictionary
            )
        }

        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.encodeFailed(ext.uppercased())
        }
        return url
    }
}
