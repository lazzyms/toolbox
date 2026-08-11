import CoreGraphics
import Foundation
import Testing
@testable import ToolboxKit

/// Regression tests for the "never make it bigger" guarantee.
///
/// These exist because a real 4032×3024 HEIC exposed two bugs that synthetic
/// tiny images did not: converting HEIC→JPEG grew the file 7%, and forcing
/// lossless output to PNG grew it 246%.
@Suite("Compression size guard")
struct CompressionGuardTests {
    let processor = ImageProcessor()

    @Test("lossless compression never returns a larger file than the original")
    func losslessNeverGrows() throws {
        let fixtures = try Fixtures()
        try #require(ImageFormat.heic.canEncode)
        // Photo-like and reasonably large: this is where PNG loses badly.
        let input = try fixtures.image(named: "photo", width: 1200, height: 900, format: .heic)
        let originalBytes = OutputNaming.fileSize(of: input)

        let result = try processor.run(input, options: .init(
            format: nil,
            keepSmallerOriginal: true,
            suffix: "-compressed",
            location: .directory(fixtures.directory)
        ))

        #expect(result.newBytes <= originalBytes)
        #expect(OutputNaming.fileSize(of: result.output) <= originalBytes)
    }

    @Test("lossy compression never returns a larger file than the original")
    func lossyNeverGrows() throws {
        let fixtures = try Fixtures()
        try #require(ImageFormat.heic.canEncode)
        let input = try fixtures.image(named: "photo", width: 1200, height: 900, format: .heic)
        let originalBytes = OutputNaming.fileSize(of: input)

        // HEIC→JPEG is the pairing that grows: JPEG is far less efficient.
        let result = try processor.run(input, options: .init(
            format: .jpeg,
            quality: 0.8,
            keepSmallerOriginal: true,
            suffix: "-compressed",
            location: .directory(fixtures.directory)
        ))

        #expect(result.newBytes <= originalBytes)
    }

    @Test("fallback copy keeps an extension that matches the actual bytes")
    func fallbackExtensionIsHonest() throws {
        let fixtures = try Fixtures()
        try #require(ImageFormat.heic.canEncode)
        let input = try fixtures.image(named: "photo", width: 1200, height: 900, format: .heic)

        let result = try processor.run(input, options: .init(
            format: .jpeg,
            quality: 0.8,
            keepSmallerOriginal: true,
            suffix: "-compressed",
            location: .directory(fixtures.directory)
        ))

        // When we fall back we copy HEIC bytes, so the name must say heic —
        // never .jpg wrapping HEIC data.
        let actualType = Fixtures.format(of: result.output)
        let expected = result.keptOriginal ? ImageFormat.heic : ImageFormat.jpeg
        #expect(actualType == expected.utType.identifier)
        #expect(result.output.pathExtension == expected.fileExtension)
    }

    @Test("resizing still reports a real size change rather than falling back")
    func resizeBypassesGuard() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "wide", width: 1600, height: 1200, format: .png)

        // Downscaling must apply even if the encoder produces more bytes; the
        // pixel dimensions are what the user asked for.
        let result = try processor.run(input, options: .init(
            resize: .fit(width: 400, height: 400),
            keepSmallerOriginal: true,
            suffix: "-resized",
            location: .directory(fixtures.directory)
        ))

        #expect(result.keptOriginal == false)
        #expect(Fixtures.pixelSize(of: result.output) == CGSize(width: 400, height: 300))
    }
}
