import Testing
import Foundation
import PDFKit
@testable import ToolboxKit

@Suite("PDF Page Extractor")
struct PageExtractorTests {
    @Test func extractsSelectedPagesInOrderWithSelectableText() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "extract-basic", text: "Page", pages: 6)

        let output = try PageExtractor.extract(input, selection: "2-3", to: .alongsideInput)

        #expect(output.lastPathComponent == "extract-basic-pages.pdf")
        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 2)
        // Reading the text back proves pages were copied as vectors, not
        // rasterised — a re-render would leave `.string` empty.
        #expect((doc.page(at: 0)?.string ?? "").contains("Page 2"))
        #expect((doc.page(at: 1)?.string ?? "").contains("Page 3"))
    }

    @Test func singlesRangesAndOpenEndsCombineIntoOneSortedSelection() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "extract-mixed", text: "Page", pages: 6)

        let output = try PageExtractor.extract(input, selection: "5, 1-2, 6-", to: .alongsideInput)

        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 4)
        let texts = (0..<doc.pageCount).compactMap { doc.page(at: $0)?.string }
        #expect(texts.count == 4)
        for page in [1, 2, 5, 6] {
            #expect(texts.contains { $0.contains("Page \(page)") })
        }
        #expect(!texts.contains { $0.contains("Page 3") })
    }

    @Test func repeatedAndOverlappingPicksIncludeEachPageOnce() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "extract-dupe", text: "Page", pages: 4)

        let output = try PageExtractor.extract(input, selection: "2, 2, 1-3", to: .alongsideInput)

        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 3)
    }

    @Test func oddKeywordSelectsEveryOddPage() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "extract-odd", text: "Page", pages: 4)

        let output = try PageExtractor.extract(input, selection: "odd", to: .alongsideInput)

        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 2)
        #expect((doc.page(at: 0)?.string ?? "").contains("Page 1"))
        #expect((doc.page(at: 1)?.string ?? "").contains("Page 3"))
    }

    @Test func rejectsBlankSelection() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "extract-blank", text: "Page", pages: 2)

        #expect {
            try PageExtractor.extract(input, selection: "   ", to: .alongsideInput)
        } throws: { error in
            (error as? ToolboxError)?.errorDescription?.contains("at least one page") == true
        }
    }

    @Test func rejectsSeparatorOnlySelection() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "extract-commas", text: "Page", pages: 2)

        // Blank means "all pages" elsewhere; here it must not quietly copy
        // the whole document.
        #expect {
            try PageExtractor.extract(input, selection: " , ,", to: .alongsideInput)
        } throws: { error in
            (error as? ToolboxError)?.errorDescription?.contains("at least one page") == true
        }
    }

    @Test func rangeTypoSurfacesTheParserError() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "extract-typo", text: "Page", pages: 2)

        #expect {
            try PageExtractor.extract(input, selection: "0", to: .alongsideInput)
        } throws: { error in
            guard let toolboxError = error as? ToolboxError,
                  case .invalidPageRange = toolboxError else { return false }
            return true
        }
    }

    @Test func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "extract-collide", text: "Page", pages: 2)

        let first = try PageExtractor.extract(input, selection: "1", to: .alongsideInput)
        let second = try PageExtractor.extract(input, selection: "2", to: .alongsideInput)

        #expect(first.lastPathComponent == "extract-collide-pages.pdf")
        #expect(second.lastPathComponent == "extract-collide-pages-1.pdf")
    }

    @Test func originalIsUntouchedAfterExtracting() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "extract-intact", text: "Page", pages: 4)
        let before = try Data(contentsOf: input)

        _ = try PageExtractor.extract(input, selection: "2", to: .alongsideInput)

        let after = try Data(contentsOf: input)
        #expect(before == after)
        #expect(try PDFDocumentIO.open(input).pageCount == 4)
    }

    @Test func writesIntoAPickedDirectory() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "extract-dir", text: "Page", pages: 2)
        let folder = fixtures.directory.appendingPathComponent("chosen")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)

        let output = try PageExtractor.extract(input, selection: "1-2", to: .directory(folder))

        // Path strings, not URLs: deletingLastPathComponent yields a URL that
        // isn't `==` to an identically-built one.
        #expect(output.deletingLastPathComponent().path == folder.path)
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "extract-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PageExtractor.extract(locked, selection: "1", to: .alongsideInput)
        }
    }
}
