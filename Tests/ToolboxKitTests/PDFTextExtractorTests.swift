import Testing
import Foundation
@testable import ToolboxKit

@Suite("PDF Text Extractor")
struct PDFTextExtractorTests {
    @Test func plainTextCarriesTheDocumentText() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "text-doc", text: "Extractable words", pages: 2)

        let output = try PDFTextExtractor.extract(
            input,
            options: PDFTextOptions(style: .plainText),
            pageRangeText: nil,
            to: .alongsideInput
        )

        #expect(output.lastPathComponent == "text-doc-text.txt")
        let text = try String(contentsOf: output, encoding: .utf8)
        #expect(text.contains("Extractable words"))
    }

    @Test func pageSeparatorsAreOptional() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "sep-doc", text: "Separator page", pages: 2)

        let with = try PDFTextExtractor.extract(
            input,
            options: PDFTextOptions(includePageSeparators: true),
            pageRangeText: nil,
            to: .alongsideInput
        )
        let without = try PDFTextExtractor.extract(
            input,
            options: PDFTextOptions(includePageSeparators: false),
            pageRangeText: nil,
            to: .alongsideInput
        )

        #expect(try String(contentsOf: with, encoding: .utf8).contains("--- page 2 ---"))
        let withoutText = try String(contentsOf: without, encoding: .utf8)
        #expect(!withoutText.contains("--- page 2 ---"))
    }

    @Test func markdownInfersHeadingsFromFontSizes() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdfWithTitleAndBody(
            named: "md-doc",
            title: "Quarterly Report",
            body: "Regular body text at a much smaller size."
        )

        let output = try PDFTextExtractor.extract(
            input,
            options: PDFTextOptions(style: .markdown),
            pageRangeText: nil,
            to: .alongsideInput
        )

        #expect(output.pathExtension == "md")
        let text = try String(contentsOf: output, encoding: .utf8)
        #expect(text.contains("# Quarterly Report"))
        #expect(text.contains("Regular body text"))
    }

    @Test func respectsThePageRange() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "range-text", text: "Ranged extraction", pages: 3)

        let output = try PDFTextExtractor.extract(
            input,
            options: PDFTextOptions(includePageSeparators: true),
            pageRangeText: "1-2",
            to: .alongsideInput
        )

        let text = try String(contentsOf: output, encoding: .utf8)
        #expect(text.contains("--- page 2 ---"))
        #expect(!text.contains("page 3"))
    }

    @Test func scanWithoutTextLayerIsRefusedNotWrittenEmpty() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.imageOnlyPDF(named: "scan-only")

        #expect {
            try PDFTextExtractor.extract(
                input, options: PDFTextOptions(), pageRangeText: nil, to: .alongsideInput
            )
        } throws: { error in
            (error as? ToolboxError)?.errorDescription?.contains("scan") == true
        }
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "text-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFTextExtractor.extract(
                locked, options: PDFTextOptions(), pageRangeText: nil, to: .alongsideInput
            )
        }
    }

    @Test func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "collide-text", text: "Once")

        let first = try PDFTextExtractor.extract(
            input, options: PDFTextOptions(), pageRangeText: nil, to: .alongsideInput
        )
        let second = try PDFTextExtractor.extract(
            input, options: PDFTextOptions(), pageRangeText: nil, to: .alongsideInput
        )

        #expect(first.lastPathComponent == "collide-text-text.txt")
        #expect(second.lastPathComponent == "collide-text-text-1.txt")
    }
}
