import CoreGraphics
import Foundation
import ImageIO
import Testing
@testable import ToolboxKit

/// Pins the tone tool's own promises: a fully neutral run cannot drift from the
/// source pixels, each knob moves the image in the direction its name claims,
/// saturation zero really does flatten colour, and every failure keeps the
/// original untouched while `OutputNaming` keeps the outputs apart.
@Suite("ImageToneAdjuster")
struct ImageToneAdjusterTests {

    // MARK: - Identity

    @Test("a zero-slider run reproduces the original pixels")
    func identityRunIsVisuallyIdentical() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.threeToneImage(named: "identity")

        let output = try ImageToneAdjuster.run(input, location: .directory(fixtures.directory))

        #expect(output.lastPathComponent == "identity-adjusted.png")
        #expect(Fixtures.format(of: output) == ImageFormat.png.utType.identifier)
        let difference = Self.meanAbsoluteDifference(
            Self.pixels(of: try #require(Self.decoded(input))),
            Self.pixels(of: try #require(Self.decoded(output)))
        )
        #expect(difference < 0.25)
    }

    @Test("a rotated phone photo comes out upright and still JPEG")
    func orientedJPEGStaysJPEGAndUpright() throws {
        let fixtures = try Fixtures()
        // 40×20 stored, tagged for a quarter turn: 20×40 as displayed.
        let input = try fixtures.orientedJPEG(named: "sideways")

        let output = try ImageToneAdjuster.run(input, location: .directory(fixtures.directory))

        #expect(output.pathExtension == "jpg")
        #expect(Fixtures.pixelSize(of: output) == CGSize(width: 20, height: 40))
    }

    // MARK: - The knobs do what they say

    @Test("positive brightness raises mean luminance, negative lowers it")
    func brightnessMovesMeanLuminance() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.flatImage(named: "grey")
        let location = OutputLocation.directory(fixtures.directory)

        let baseline = Self.meanLuminance(of: try #require(Self.decoded(
            try ImageToneAdjuster.run(input, location: location)
        )))
        let brighter = Self.meanLuminance(of: try #require(Self.decoded(
            try ImageToneAdjuster.run(input, options: .init(brightness: 0.3), location: location)
        )))
        let darker = Self.meanLuminance(of: try #require(Self.decoded(
            try ImageToneAdjuster.run(input, options: .init(brightness: -0.3), location: location)
        )))

        #expect(brighter > baseline)
        #expect(darker < baseline)
    }

    @Test("exposure raises mean luminance")
    func exposureBrightens() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.flatImage(named: "grey")
        let location = OutputLocation.directory(fixtures.directory)

        let baseline = Self.meanLuminance(of: try #require(Self.decoded(
            try ImageToneAdjuster.run(input, location: location)
        )))
        let brighter = Self.meanLuminance(of: try #require(Self.decoded(
            try ImageToneAdjuster.run(input, options: .init(exposure: 1), location: location)
        )))

        #expect(brighter > baseline)
    }

    @Test("saturation zero collapses colour to gray")
    func zeroSaturationCollapsesToGrayscale() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.threeToneImage(named: "tone")

        let output = try ImageToneAdjuster.run(
            input, options: .init(saturation: 0), location: .directory(fixtures.directory)
        )
        let image = try #require(Self.decoded(output))
        let data = Self.pixels(of: image)

        // One sample in each third — red, green, blue — must now have all
        // three channels equal.
        for x in [10, 30, 50] {
            let offset = (10 * image.width + x) * 4
            let red = data[offset]
            let green = data[offset + 1]
            let blue = data[offset + 2]
            #expect(red == green && green == blue)
        }
    }

    @Test("a shifted temperature really reaches the pixels")
    func temperatureChangesPixels() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.flatImage(named: "grey")
        let location = OutputLocation.directory(fixtures.directory)

        let neutral = try ImageToneAdjuster.run(input, location: location)
        let shifted = try ImageToneAdjuster.run(
            input, options: .init(temperature: 3500), location: location
        )

        let difference = Self.meanAbsoluteDifference(
            Self.pixels(of: try #require(Self.decoded(neutral))),
            Self.pixels(of: try #require(Self.decoded(shifted)))
        )
        #expect(difference > 0.5)
    }

    // MARK: - File safety

    @Test("garbage input fails the file with a decode failure")
    func corruptInputFailsAsDecodeFailure() throws {
        let fixtures = try Fixtures()
        let bad = fixtures.directory.appendingPathComponent("broken").appendingPathExtension("png")
        try Data([0xDE, 0xAD, 0xBE, 0xEF]).write(to: bad)

        #expect(throws: ToolboxError.decodeFailed(bad)) {
            try ImageToneAdjuster.run(bad, location: .directory(fixtures.directory))
        }
    }

    @Test("the original is never modified and never overwritten")
    func originalsAreUntouchedAndOutputsNeverCollide() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.threeToneImage(named: "keep")
        let before = try Data(contentsOf: input)
        let location = OutputLocation.directory(fixtures.directory)

        let first = try ImageToneAdjuster.run(
            input, options: .init(saturation: 0), location: location
        )
        let second = try ImageToneAdjuster.run(
            input, options: .init(saturation: 0), location: location
        )

        #expect(first.lastPathComponent == "keep-adjusted.png")
        #expect(second.lastPathComponent == "keep-adjusted-1.png")
        #expect(try Data(contentsOf: input) == before)
    }

    // MARK: - Reading pixels back

    private static func decoded(_ url: URL) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(source, 0, nil)
    }

    /// Canonical sRGB RGBA bytes, top row first, so two images are comparable.
    private static func pixels(of image: CGImage) -> [UInt8] {
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
        return bytes
    }

    private static func meanAbsoluteDifference(_ first: [UInt8], _ second: [UInt8]) -> Double {
        precondition(first.count == second.count)
        guard !first.isEmpty else { return 0 }
        var total = 0
        for index in first.indices {
            total += abs(Int(first[index]) - Int(second[index]))
        }
        return Double(total) / Double(first.count)
    }

    private static func meanLuminance(of image: CGImage) -> Double {
        let data = pixels(of: image)
        guard !data.isEmpty else { return 0 }
        var total = 0
        var index = 0
        while index + 2 < data.count {
            total += Int(data[index]) + Int(data[index + 1]) + Int(data[index + 2])
            index += 4
        }
        return Double(total) / (Double(data.count) / 4 * 3)
    }
}
