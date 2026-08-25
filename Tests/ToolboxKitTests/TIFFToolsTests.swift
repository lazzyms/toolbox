import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import ToolboxKit

/// The two multi-page TIFF tools: pages → files, and files → one multi-page
/// file. Pages have no timing, so the checks are about count, order and
/// never touching the originals.
@Suite("TIFF tools")
struct TIFFToolsTests {

    // MARK: - Split

    @Test("split writes every page as its own file, in page order")
    func splitWritesEveryPageInPageOrder() throws {
        try #require(ImageFormat.png.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.multipageTIFF(named: "scan", frames: 3)
        let originalBytes = OutputNaming.fileSize(of: input)

        let outputs = try TIFFTools.split(
            input, format: .png, to: .directory(fixtures.directory)
        )

        #expect(outputs.count == 3)
        for (index, output) in outputs.enumerated() {
            #expect(output.lastPathComponent == "scan-frame-\(index + 1).png")
            #expect(Fixtures.pixelSize(of: output) == CGSize(width: 32, height: 24))
            #expect(OutputNaming.fileSize(of: output) > 0)
        }

        // The fixture's pages are red, green, blue — order must survive.
        let expected: [RGB] = [.init(255, 0, 0), .init(0, 255, 0), .init(0, 0, 255)]
        for (index, output) in outputs.enumerated() {
            let colour = try #require(Self.pixelColour(of: output, x: 0, y: 0))
            #expect(colour == expected[index])
        }

        // The multi-page TIFF is never modified.
        #expect(OutputNaming.fileSize(of: input) == originalBytes)
    }

    @Test("split can keep the pages as TIFF")
    func splitCanKeepTIFFOutput() throws {
        try #require(ImageFormat.tiff.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.multipageTIFF(named: "fax", frames: 2)

        let outputs = try TIFFTools.split(
            input, format: .tiff, to: .directory(fixtures.directory)
        )

        #expect(outputs.count == 2)
        #expect(outputs.allSatisfy { $0.pathExtension == "tiff" })
        #expect(Fixtures.format(of: outputs[0]) == UTType.tiff.identifier)
    }

    @Test("split honours a per-page size")
    func splitHonoursPerPageSize() throws {
        try #require(ImageFormat.png.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.multipageTIFF(named: "mixed", frames: 2, sizes: [
            CGSize(width: 40, height: 20),
            CGSize(width: 10, height: 30),
        ])

        let outputs = try TIFFTools.split(input, format: .png, to: .directory(fixtures.directory))

        #expect(Fixtures.pixelSize(of: outputs[0]) == CGSize(width: 40, height: 20))
        #expect(Fixtures.pixelSize(of: outputs[1]) == CGSize(width: 10, height: 30))
    }

    @Test("splitting a single-page TIFF produces exactly one file")
    func singlePageSplitProducesOneFile() throws {
        try #require(ImageFormat.png.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.multipageTIFF(named: "lone", frames: 1)

        let outputs = try TIFFTools.split(input, format: .png, to: .directory(fixtures.directory))

        #expect(outputs.count == 1)
        #expect(outputs[0].lastPathComponent == "lone-frame-1.png")
    }

    @Test("rerunning split numbers colliding outputs instead of overwriting")
    func rerunSplitNumbersCollisions() throws {
        try #require(ImageFormat.png.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.multipageTIFF(named: "again", frames: 2)

        let first = try TIFFTools.split(input, format: .png, to: .directory(fixtures.directory))
        let second = try TIFFTools.split(input, format: .png, to: .directory(fixtures.directory))

        #expect(first[0].lastPathComponent == "again-frame-1.png")
        #expect(second[0].lastPathComponent == "again-frame-1-1.png")
        #expect(second.allSatisfy { OutputNaming.fileSize(of: $0) > 0 })
        // Nothing was lost or replaced.
        #expect(try Self.contents(of: fixtures.directory).contains("again-frame-2.png"))
    }

    // MARK: - Combine

    @Test("combine builds a multi-page TIFF in queue order")
    func combineBuildsMultiPageTIFFInQueueOrder() throws {
        try #require(ImageFormat.tiff.canEncode)
        let fixtures = try Fixtures()
        let inputs = [
            try Self.solidImage(named: "blue", in: fixtures.directory, red: 0, green: 0, blue: 1),
            try Self.solidImage(named: "green", in: fixtures.directory, red: 0, green: 1, blue: 0),
            try Self.solidImage(named: "red", in: fixtures.directory, red: 1, green: 0, blue: 0),
        ]
        let originalBytes = inputs.map { OutputNaming.fileSize(of: $0) }

        let output = try TIFFTools.combine(inputs, to: .directory(fixtures.directory))

        #expect(output.lastPathComponent == "blue-combined.tiff")
        #expect(Fixtures.format(of: output) == UTType.tiff.identifier)
        #expect(Self.frameCount(of: output) == 3)
        #expect(Self.frameSize(of: output, at: 0) == CGSize(width: 8, height: 8))

        let expected: [RGB] = [.init(0, 0, 255), .init(0, 255, 0), .init(255, 0, 0)]
        for index in 0..<3 {
            let colour = try #require(Self.pixelColour(of: output, atFrame: index, x: 0, y: 0))
            #expect(colour == expected[index])
        }

        // The inputs are never modified.
        for (index, input) in inputs.enumerated() {
            #expect(OutputNaming.fileSize(of: input) == originalBytes[index])
        }
    }

    @Test("combining a single image yields a legitimate one-page TIFF")
    func combiningSingleImageYieldsOnePageTIFF() throws {
        try #require(ImageFormat.tiff.canEncode)
        let fixtures = try Fixtures()
        let input = try Self.solidImage(named: "solo", in: fixtures.directory, red: 1, green: 0, blue: 0)

        let output = try TIFFTools.combine([input], to: .directory(fixtures.directory))

        #expect(output.lastPathComponent == "solo-combined.tiff")
        #expect(Self.frameCount(of: output) == 1)
    }

    @Test("rerunning combine numbers colliding outputs instead of overwriting")
    func rerunCombineNumbersCollisions() throws {
        try #require(ImageFormat.tiff.canEncode)
        let fixtures = try Fixtures()
        let inputs = [
            try Self.solidImage(named: "one", in: fixtures.directory, red: 1, green: 1, blue: 1),
        ]

        let first = try TIFFTools.combine(inputs, to: .directory(fixtures.directory))
        let second = try TIFFTools.combine(inputs, to: .directory(fixtures.directory))

        #expect(first.lastPathComponent == "one-combined.tiff")
        #expect(second.lastPathComponent == "one-combined-1.tiff")
        #expect(OutputNaming.fileSize(of: first) > 0)
    }

    @Test("an undecodable image fails the whole combine with nothing written")
    func undecodableInputThrowsDecodeFailed() throws {
        let fixtures = try Fixtures()
        let broken = fixtures.directory.appendingPathComponent("broken.png")
        try Data("definitely not image data".utf8).write(to: broken)
        let good = try Self.solidImage(named: "fine", in: fixtures.directory, red: 0, green: 1, blue: 0)
        let before = try Self.contents(of: fixtures.directory)

        #expect(throws: ToolboxError.decodeFailed(broken)) {
            _ = try TIFFTools.combine([good, broken], to: .directory(fixtures.directory))
        }

        // A refusal writes nothing at all.
        #expect(try Self.contents(of: fixtures.directory) == before)
    }

    @Test("a non-image file is refused by extension")
    func nonImageInputIsRefused() throws {
        let fixtures = try Fixtures()
        let pdf = try fixtures.pdf(named: "doc")

        #expect(throws: ToolboxError.unsupportedInput("pdf")) {
            _ = try TIFFTools.combine([pdf], to: .directory(fixtures.directory))
        }
    }

    @Test("empty selection is refused with a clear message")
    func emptySelectionIsRefused() throws {
        #expect(throws: ToolboxError.emptySelection) {
            _ = try TIFFTools.combine([], to: .alongsideInput)
        }
    }

    // MARK: - Round trip

    @Test("split of a combined TIFF returns the queued images in order")
    func splitRoundTripsCombinedPages() throws {
        try #require(ImageFormat.tiff.canEncode)
        try #require(ImageFormat.png.canEncode)
        let fixtures = try Fixtures()
        let inputs = [
            try Self.solidImage(named: "r", in: fixtures.directory, red: 1, green: 0, blue: 0),
            try Self.solidImage(named: "g", in: fixtures.directory, red: 0, green: 1, blue: 0),
            try Self.solidImage(named: "b", in: fixtures.directory, red: 0, green: 0, blue: 1),
        ]

        let combined = try TIFFTools.combine(inputs, to: .directory(fixtures.directory))
        let split = try TIFFTools.split(combined, format: .png, to: .directory(fixtures.directory))

        #expect(split.count == 3)
        #expect(split.allSatisfy { $0.pathExtension == "png" })
        let colours = try split.map { url in
            try #require(Self.pixelColour(of: url, x: 0, y: 0))
        }
        #expect(colours == [.init(255, 0, 0), .init(0, 255, 0), .init(0, 0, 255)])
    }

    // MARK: - Helpers

    /// A solid-colour PNG, so a single sampled pixel identifies an image.
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

    /// The RGB triple of a single pixel of frame `index`, rendered at native
    /// size so any coordinate can be sampled.
    private struct RGB: Equatable {
        let r, g, b: UInt8
        init(_ r: UInt8, _ g: UInt8, _ b: UInt8) { self.r = r; self.g = g; self.b = b }
    }

    private static func pixelColour(
        of url: URL, atFrame index: Int = 0, x: Int, y: Int
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

    private static func contents(of directory: URL) throws -> Set<String> {
        Set(try FileManager.default.contentsOfDirectory(atPath: directory.path))
    }
}
