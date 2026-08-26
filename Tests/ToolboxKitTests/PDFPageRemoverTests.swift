import Testing
import Foundation
import PDFKit
@testable import ToolboxKit

@Suite("PDF Page Remover")
struct PDFPageRemoverTests {

    private static func pageTexts(of doc: PDFDocument) -> [String] {
        (0..<doc.pageCount).compactMap { doc.page(at: $0)?.string }
    }

    @Test func removesTheSelectionKeepingSurvivorsInOrder() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "trim-basic", text: "Page", pages: 8)

        let output = try PDFPageRemover.remove(
            input, options: PDFPageRemoveOptions(pages: "2, 5-7"), to: .alongsideInput
        )

        #expect(output.lastPathComponent == "trim-basic-trimmed.pdf")
        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 4)
        #expect(Self.pageTexts(of: doc) == ["Page 1", "Page 3", "Page 4", "Page 8"])
    }

    @Test func outputPageCountIsInputMinusSelection() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "trim-count", text: "Page", pages: 6)

        let output = try PDFPageRemover.remove(
            input, options: PDFPageRemoveOptions(pages: "odd"), to: .alongsideInput
        )

        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 3)
        #expect(Self.pageTexts(of: doc) == ["Page 2", "Page 4", "Page 6"])
    }

    @Test func openEndedRangeTrimsThroughToTheEnd() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "trim-open-ended", text: "Page", pages: 6)

        let output = try PDFPageRemover.remove(
            input, options: PDFPageRemoveOptions(pages: "5-"), to: .alongsideInput
        )

        let doc = try PDFDocumentIO.open(output)
        #expect(Self.pageTexts(of: doc) == ["Page 1", "Page 2", "Page 3", "Page 4"])
    }

    @Test func refusesToRemoveEveryPage() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "trim-everything", text: "Page", pages: 3)

        #expect(throws: ToolboxError.removesAllPages(input)) {
            try PDFPageRemover.remove(
                input, options: PDFPageRemoveOptions(pages: "1-3"), to: .alongsideInput
            )
        }
    }

    @Test func refusesABlankSelectionWhichMeansEverything() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "trim-blank", text: "Page", pages: 3)

        #expect {
            try PDFPageRemover.remove(
                input, options: PDFPageRemoveOptions(pages: "  "), to: .alongsideInput
            )
        } throws: { error in
            guard case .removesAllPages = error as? ToolboxError else {
                return false
            }
            return true
        }
    }

    @Test func fileShorterThanSelectionFailsWithAPageNumber() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "trim-short", text: "Page", pages: 2)

        #expect {
            try PDFPageRemover.remove(
                input, options: PDFPageRemoveOptions(pages: "5"), to: .alongsideInput
            )
        } throws: { error in
            (error as? ToolboxError)?.errorDescription?.contains("outside") == true
        }
    }

    @Test func rangeTypoSurfacesTheParserError() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "trim-typo", text: "Page", pages: 2)

        #expect {
            try PDFPageRemover.remove(
                input, options: PDFPageRemoveOptions(pages: "0"), to: .alongsideInput
            )
        } throws: { error in
            guard let toolboxError = error as? ToolboxError,
                  case .invalidPageRange = toolboxError else { return false }
            return true
        }
    }

    @Test func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "trim-collide", text: "Page", pages: 2)

        let first = try PDFPageRemover.remove(
            input, options: PDFPageRemoveOptions(pages: "1"), to: .alongsideInput
        )
        let second = try PDFPageRemover.remove(
            input, options: PDFPageRemoveOptions(pages: "1"), to: .alongsideInput
        )

        #expect(first.lastPathComponent == "trim-collide-trimmed.pdf")
        #expect(second.lastPathComponent == "trim-collide-trimmed-1.pdf")
    }

    @Test func writesIntoAPickedDirectory() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "trim-dir", text: "Page", pages: 2)
        let folder = fixtures.directory.appendingPathComponent("chosen")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)

        let output = try PDFPageRemover.remove(
            input, options: PDFPageRemoveOptions(pages: "2"), to: .directory(folder)
        )

        // Path string, not URL: deletingLastPathComponent yields a URL that
        // isn't `==` to an identically-built one.
        #expect(output.deletingLastPathComponent().path == folder.path)
    }

    @Test func originalIsUntouchedAfterRemoval() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "trim-intact", text: "Page", pages: 4)
        let before = try Data(contentsOf: input)

        _ = try PDFPageRemover.remove(
            input, options: PDFPageRemoveOptions(pages: "1-2"), to: .alongsideInput
        )

        #expect(try Data(contentsOf: input) == before)
        let doc = try PDFDocumentIO.open(input)
        #expect(doc.pageCount == 4)
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "trim-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFPageRemover.remove(
                locked, options: PDFPageRemoveOptions(pages: "1"), to: .alongsideInput
            )
        }
    }
}
