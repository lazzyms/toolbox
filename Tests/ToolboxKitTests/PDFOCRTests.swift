import Testing
import Foundation
@testable import ToolboxKit

@Suite("PDF OCR")
struct PDFOCRTests {
    @Test func ocrReadsTextOutOfRenderedPixels() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.scannedTextPDF(named: "scan-doc", text: "TOOLBOX")

        let output = try PDFOCR.recognize(
            input, options: PDFOCROptions(), pageRangeText: nil, to: .alongsideInput
        )

        #expect(output.lastPathComponent == "scan-doc-ocr-text.txt")
        let text = try String(contentsOf: output, encoding: .utf8)
        // Vision is stochastic at the margins, so assert one distinctive
        // token, case- and whitespace-normalised, not a whole sentence.
        #expect(normalised(text).contains("toolbox"))
    }

    /// Vision's output spacing is unpredictable, so comparisons run against
    /// words separated by single spaces, lowercased.
    private func normalised(_ text: String) -> String {
        text.lowercased()
            .components(separatedBy: CharacterSet.alphanumerics.inverted)
            .joined(separator: " ")
    }

    @Test func blankScanWritesAnEmptyFileInsteadOfFailing() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.imageOnlyPDF(named: "scan-blank")

        let output = try PDFOCR.recognize(
            input, options: PDFOCROptions(), pageRangeText: nil, to: .alongsideInput
        )

        // Unlike the text extractor, an empty result is legitimate here — the
        // file must still be written so the batch reports success.
        #expect(FileManager.default.fileExists(atPath: output.path))
    }

    @Test func respectsThePageRange() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.scannedTextPDF(named: "scan-range", text: "TOOLBOX", pages: 3)

        let output = try PDFOCR.recognize(
            input,
            options: PDFOCROptions(includePageSeparators: true),
            pageRangeText: "1-2",
            to: .alongsideInput
        )

        let text = try String(contentsOf: output, encoding: .utf8)
        #expect(text.contains("--- page 2 ---"))
        #expect(!text.contains("--- page 3 ---"))
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "ocr-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFOCR.recognize(
                locked, options: PDFOCROptions(), pageRangeText: nil, to: .alongsideInput
            )
        }
    }

    @Test func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.scannedTextPDF(named: "collide-scan", text: "TOOLBOX")

        let first = try PDFOCR.recognize(
            input, options: PDFOCROptions(), pageRangeText: nil, to: .alongsideInput
        )
        let second = try PDFOCR.recognize(
            input, options: PDFOCROptions(), pageRangeText: nil, to: .alongsideInput
        )

        #expect(first.lastPathComponent == "collide-scan-ocr-text.txt")
        #expect(second.lastPathComponent == "collide-scan-ocr-text-1.txt")
    }
}
