import CoreGraphics
import Foundation
import ImageIO
import Testing
@testable import ToolboxKit

@Suite("ImageProcessor")
struct ImageProcessorTests {
    let processor = ImageProcessor()

    @Test("converts HEIC to PNG")
    func heicToPNG() throws {
        let fixtures = try Fixtures()
        // Skip if this Mac can't produce a HEIC to test against.
        try #require(ImageFormat.heic.canEncode)
        let input = try fixtures.image(named: "photo", width: 320, height: 240, format: .heic)

        let result = try processor.run(
            input,
            options: .init(format: .png, location: .directory(fixtures.directory))
        )

        #expect(result.output.pathExtension == "png")
        #expect(FileManager.default.fileExists(atPath: result.output.path))
        #expect(Fixtures.format(of: result.output) == ImageFormat.png.utType.identifier)
        #expect(result.pixelSize == CGSize(width: 320, height: 240))
    }

    @Test("resize writes the requested pixel dimensions")
    func resizeDimensions() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "big", width: 800, height: 600)

        let result = try processor.run(
            input,
            options: .init(
                resize: .fit(width: 400, height: 400),
                location: .directory(fixtures.directory)
            )
        )

        #expect(Fixtures.pixelSize(of: result.output) == CGSize(width: 400, height: 300))
        #expect(result.originalPixelSize == CGSize(width: 800, height: 600))
    }

    @Test("exact resize overrides aspect ratio")
    func exactResize() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "square", width: 800, height: 600)

        let result = try processor.run(
            input,
            options: .init(
                resize: .exact(width: 300, height: 300),
                location: .directory(fixtures.directory)
            )
        )

        #expect(Fixtures.pixelSize(of: result.output) == CGSize(width: 300, height: 300))
    }

    @Test("lossy quality actually reduces file size")
    func lossyShrinks() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "detail", width: 900, height: 700)

        let high = try processor.run(input, options: .init(
            format: .jpeg, quality: 0.95, suffix: "-high",
            location: .directory(fixtures.directory)
        ))
        let low = try processor.run(input, options: .init(
            format: .jpeg, quality: 0.3, suffix: "-low",
            location: .directory(fixtures.directory)
        ))

        #expect(low.newBytes < high.newBytes)
        #expect(low.savedFraction > 0)
    }

    @Test("does not overwrite an existing output")
    func noOverwrite() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "dup", width: 100, height: 100, format: .jpeg)

        let first = try processor.run(input, options: .init(
            format: .png, location: .directory(fixtures.directory)
        ))
        let second = try processor.run(input, options: .init(
            format: .png, location: .directory(fixtures.directory)
        ))

        #expect(first.output != second.output)
        #expect(second.output.lastPathComponent == "dup-1.png")
        #expect(FileManager.default.fileExists(atPath: first.output.path))
    }

    @Test("strips EXIF when asked")
    func stripsMetadata() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "meta", width: 200, height: 150, format: .jpeg)

        let result = try processor.run(input, options: .init(
            format: .jpeg, stripMetadata: true, suffix: "-clean",
            location: .directory(fixtures.directory)
        ))

        let source = CGImageSourceCreateWithURL(result.output as CFURL, nil)!
        let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        #expect(props?[kCGImagePropertyGPSDictionary] == nil)
    }

    @Test("keepSmallerOriginal falls back rather than growing a file")
    func keepsSmallerOriginal() throws {
        let fixtures = try Fixtures()
        // A tiny already-optimal PNG is the case where re-encoding tends to grow.
        let input = try fixtures.image(named: "tiny", width: 8, height: 8, format: .png)
        let originalBytes = OutputNaming.fileSize(of: input)

        let result = try processor.run(input, options: .init(
            format: .png, keepSmallerOriginal: true, suffix: "-c",
            location: .directory(fixtures.directory)
        ))

        // Either it genuinely shrank, or we kept the original bytes — never bigger.
        #expect(result.newBytes <= originalBytes)
    }

    @Test("rejects unsupported input types")
    func rejectsUnsupported() throws {
        let fixtures = try Fixtures()
        let bogus = fixtures.directory.appendingPathComponent("notes.txt")
        try "hello".write(to: bogus, atomically: true, encoding: .utf8)

        #expect(throws: ToolboxError.unsupportedInput("txt")) {
            try processor.run(bogus, options: .init())
        }
    }

    @Test("reports a clear error for corrupt image data")
    func corruptData() throws {
        let fixtures = try Fixtures()
        let corrupt = fixtures.directory.appendingPathComponent("broken.png")
        try Data([0x01, 0x02, 0x03, 0x04]).write(to: corrupt)

        #expect(throws: ToolboxError.decodeFailed(corrupt)) {
            try processor.run(corrupt, options: .init())
        }
    }
}
