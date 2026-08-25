import Testing
import Foundation
import PDFKit
@testable import ToolboxKit

@Suite("PDF Compressor")
struct PDFCompressorTests {

    @Test func shrinksAnImageHeavyPDFAndKeepsPageCount() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.noisyGradientPDF(named: "scan-doc", pages: 2)
        let originalBytes = OutputNaming.fileSize(of: input)

        let result = try PDFCompressor.compress(
            input, options: PDFCompressOptions(dpi: 150, quality: 0.75), to: .alongsideInput
        )

        #expect(result.pageCount == 2)
        #expect(result.originalBytes == originalBytes)
        #expect(result.newBytes < originalBytes)

        // A real JPEG round-trip sheds far more than half; a fallback that
        // merely re-embedded lossless pixels could squeak under "smaller"
        // while doing no honest compression.
        #expect(result.newBytes <= originalBytes / 2)

        // Page sizes unchanged: 400×200 pt in, same out.
        let doc = try PDFDocumentIO.open(result.output)
        #expect(doc.pageCount == 2)
        let box = try #require(doc.page(at: 0)?.bounds(for: .mediaBox))
        #expect(abs(box.width - 400) < 0.5)
        #expect(abs(box.height - 200) < 0.5)
    }

    @Test func outputCarriesTheCompressedSuffixAndCollisionsAreNumbered() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.noisyGradientPDF(named: "collide-compress")

        let first = try PDFCompressor.compress(
            input, options: PDFCompressOptions(dpi: 72), to: .alongsideInput
        )
        let second = try PDFCompressor.compress(
            input, options: PDFCompressOptions(dpi: 72), to: .alongsideInput
        )

        #expect(first.output.lastPathComponent == "collide-compress-compressed.pdf")
        #expect(second.output.lastPathComponent == "collide-compress-compressed-1.pdf")
    }

    @Test func textOnlyPDFFailsWithNoGain() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "text-doc", text: "Mostly vector text here")

        #expect(throws: ToolboxError.noGain) {
            try PDFCompressor.compress(
                input, options: PDFCompressOptions(dpi: 600, quality: 1.0), to: .alongsideInput
            )
        }
        // The refused copy must not linger on disk.
        #expect(!FileManager.default.fileExists(
            atPath: input.deletingLastPathComponent()
                .appendingPathComponent("text-doc-compressed.pdf").path
        ))
    }

    @Test func dpiCapThrowsResolutionTooLarge() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "poster-doc", text: "Poster")

        #expect(throws: ToolboxError.resolutionTooLarge(33333, 16667)) {
            try PDFCompressor.compress(
                input, options: PDFCompressOptions(dpi: 6000), to: .alongsideInput
            )
        }
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "compress-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFCompressor.compress(
                locked, options: PDFCompressOptions(), to: .alongsideInput
            )
        }
    }

    @Test func originalIsByteIdenticalAfterCompression() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.noisyGradientPDF(named: "untouched-doc")
        let before = try Data(contentsOf: input)

        _ = try PDFCompressor.compress(
            input, options: PDFCompressOptions(dpi: 150), to: .alongsideInput
        )

        let after = try Data(contentsOf: input)
        #expect(before == after)
    }
}

