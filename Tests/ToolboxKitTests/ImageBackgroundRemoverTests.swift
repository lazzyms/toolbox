import CoreGraphics
import Foundation
import ImageIO
import Testing
@testable import ToolboxKit

/// Pins what can honestly be pinned about subject cutouts: the deterministic
/// mask-application core does exactly what a hand-built mask says, file naming
/// and safety invariants hold, animated inputs are refused, and the real
/// Vision pipeline either cuts the subject or reports that it found none —
/// never crashes, never writes something misshapen.
///
/// Vision segmentation on synthetic images is unpredictable by nature, so no
/// test asserts what the model *must* find; they assert what the tool must do
/// with whatever the model reports.
@Suite("ImageBackgroundRemover")
struct ImageBackgroundRemoverTests {

    // MARK: - The deterministic core

    @Test("a hand-built mask keeps its side and clears the other")
    func handMaskAppliesAsAlpha() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.threeToneImage(named: "masked")

        let image = try #require(Self.decoded(input))
        let output = try ImageBackgroundRemover.compose(
            image: image, mask: Self.halfMask(width: image.width, height: image.height)
        )

        let pixels = try Self.pixels(of: output)
        let original = try Fixtures.pixels(of: input)
        let midY = pixels.height / 2

        // Probe inside each half, away from the seam at exactly width / 2.
        let kept = pixels.pixel(atX: pixels.width / 4, y: midY)
        let cleared = pixels.pixel(atX: pixels.width * 3 / 4, y: midY)
        let reference = original.pixel(atX: original.width / 4, y: midY)

        #expect(kept.a >= 253)
        #expect(cleared.a == 0)
        // A small colour drift is tolerated because CoreImage works in a linear
        // space and renders back to sRGB; the claim under test is the alpha.
        #expect(abs(Int(kept.r) - Int(reference.r)) <= 3)
        #expect(abs(Int(kept.g) - Int(reference.g)) <= 3)
        #expect(abs(Int(kept.b) - Int(reference.b)) <= 3)
    }

    @Test("soft mask edges survive as partial transparency")
    func softEdgesStaySoft() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.threeToneImage(named: "soft")
        let image = try #require(Self.decoded(input))

        // A uniform mid-grey mask means every pixel blends halfway — the
        // output must not snap to either extreme, because thresholding is
        // exactly what scissor-cuts hair and fur.
        let output = try ImageBackgroundRemover.compose(
            image: image,
            mask: Self.mask(width: image.width, height: image.height) { _, _ in .midGrey }
        )

        let alpha = try Self.pixels(of: output).pixel(atX: image.width / 2, y: image.height / 2).a
        #expect(alpha > 10 && alpha < 245)
    }

    @Test("cutout files are PNG, suffixed -cutout, and never collide")
    func writeCutoutNamesAndSafety() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.threeToneImage(named: "keep")
        let before = try Data(contentsOf: input)
        let location = OutputLocation.directory(fixtures.directory)
        let image = try #require(Self.decoded(input))

        let first = try ImageBackgroundRemover.writeCutout(
            image: try Self.halfCutout(image), for: input, in: location
        )
        let second = try ImageBackgroundRemover.writeCutout(
            image: try Self.halfCutout(image), for: input, in: location
        )

        #expect(first.lastPathComponent == "keep-cutout.png")
        #expect(second.lastPathComponent == "keep-cutout-1.png")
        #expect(Fixtures.format(of: first) == ImageFormat.png.utType.identifier)
        #expect(try Data(contentsOf: input) == before)
    }

    @Test("the written PNG carries its cleared region as real transparency")
    func writtenPNGKeepsTransparency() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.threeToneImage(named: "alpha")
        let image = try #require(Self.decoded(input))

        let output = try ImageBackgroundRemover.writeCutout(
            image: try Self.halfCutout(image),
            for: input,
            in: OutputLocation.directory(fixtures.directory)
        )

        let pixels = try Fixtures.pixels(of: output)
        #expect(pixels.pixel(atX: pixels.width * 3 / 4, y: pixels.height / 2).a == 0)
        #expect(pixels.pixel(atX: pixels.width / 4, y: pixels.height / 2).a == 255)
    }

    // MARK: - Refusals before any pixels move

    @Test("animated inputs are rejected rather than silently flattened")
    func animatedInputIsRefused() throws {
        let fixtures = try Fixtures()
        let gif = try fixtures.animatedGIF(named: "spin")

        #expect(throws: ToolboxError.wouldDropFrames(gif, frames: 3, format: "GIF")) {
            try ImageBackgroundRemover.run(gif, location: .directory(fixtures.directory))
        }
    }

    @Test("unreadable extensions are turned away")
    func unsupportedInputIsRejected() throws {
        let fixtures = try Fixtures()
        let text = fixtures.directory.appendingPathComponent("notes.txt")
        try Data("not an image".utf8).write(to: text)

        #expect(throws: ToolboxError.unsupportedInput("txt")) {
            try ImageBackgroundRemover.run(text, location: .directory(fixtures.directory))
        }
    }

    @Test("garbage input fails the file with a decode failure")
    func corruptInputFailsAsDecodeFailure() throws {
        let fixtures = try Fixtures()
        let bad = fixtures.directory.appendingPathComponent("broken").appendingPathExtension("png")
        try Data([0xDE, 0xAD, 0xBE, 0xEF]).write(to: bad)

        #expect(throws: ToolboxError.decodeFailed(bad)) {
            try ImageBackgroundRemover.run(bad, location: .directory(fixtures.directory))
        }
    }

    // MARK: - Real Vision smoke

    @Test("Vision either produces a correctly-shaped cutout or admits defeat")
    func visionPipelineStaysHonest() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.highContrastSubjectImage(named: "subject")
        let declaredSize = Fixtures.pixelSize(of: input)
        let before = try Data(contentsOf: input)

        let output: URL
        do {
            output = try ImageBackgroundRemover.run(input, location: .directory(fixtures.directory))
        } catch let error as ToolboxError {
            // Finding nothing in a synthetic drawing is a legitimate verdict;
            // any other error leaking out of the pipeline would be a bug.
            #expect(error == .noSubjectFound(input))
            return
        }

        #expect(Fixtures.format(of: output) == ImageFormat.png.utType.identifier)
        if let size = declaredSize {
            #expect(Fixtures.pixelSize(of: output) == size)
        }
        try #expect(Data(contentsOf: input) == before)
    }

    // MARK: - Helpers

    private static func halfCutout(_ image: CGImage) throws -> CGImage {
        try ImageBackgroundRemover.compose(
            image: image, mask: halfMask(width: image.width, height: image.height)
        )
    }

    /// White across the displayed left half, black across the right.
    private static func halfMask(width: Int, height: Int) -> CGImage {
        mask(width: width, height: height) { x, _ in x < width / 2 ? .white : .black }
    }

    private enum Shade {
        case white, black, midGrey

        var level: CGFloat {
            switch self {
            case .white: return 1
            case .black: return 0
            case .midGrey: return 0.5
            }
        }
    }

    /// Builds an in-memory grayscale mask; `shade` maps display coordinates
    /// (origin top-left, matching how the fixtures name their regions) to a
    /// level, so a hand-built mask reads like the assertion about it.
    private static func mask(
        width: Int, height: Int, shade: (_ x: Int, _ y: Int) -> Shade
    ) -> CGImage {
        let context = CGContext(
            data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        // CGContext puts the origin at the bottom left; flip once so the
        // closure can be written in display coordinates throughout.
        context.translateBy(x: 0, y: CGFloat(height))
        context.scaleBy(x: 1, y: -1)
        for y in 0..<height {
            for x in 0..<width {
                context.setFillColor(CGColor(gray: shade(x, y).level, alpha: 1))
                context.fill(CGRect(x: x, y: y, width: 1, height: 1))
            }
        }
        return context.makeImage()!
    }

    private static func decoded(_ url: URL) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(source, 0, nil)
    }

    /// Canonical RGBA8 sRGB bytes for an in-memory image, same layout
    /// `Fixtures.pixels` produces for files, so probes read the same way.
    private static func pixels(of image: CGImage) throws -> Fixtures.PixelBuffer {
        var bytes = [UInt8](repeating: 0, count: image.width * image.height * 4)
        bytes.withUnsafeMutableBytes { raw in
            guard let context = CGContext(
                data: raw.baseAddress, width: image.width, height: image.height,
                bitsPerComponent: 8, bytesPerRow: image.width * 4,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else { return }
            context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        }
        return Fixtures.PixelBuffer(bytes: bytes, width: image.width, height: image.height)
    }
}
