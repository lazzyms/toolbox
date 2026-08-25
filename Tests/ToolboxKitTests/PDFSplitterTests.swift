import Testing
import Foundation
import PDFKit
@testable import ToolboxKit

@Suite("PDF Splitter")
struct PDFSplitterTests {
    @Test func everyPageWritesOneFilePerPage() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "split-all", text: "Page", pages: 3)

        let outputs = try PDFSplitter.split(
            input, options: PDFSplitOptions(mode: .everyPage), to: .alongsideInput
        )

        #expect(
            outputs.map(\.lastPathComponent)
                == ["split-all-1.pdf", "split-all-2.pdf", "split-all-3.pdf"]
        )
        for (index, output) in outputs.enumerated() {
            let doc = try PDFDocumentIO.open(output)
            #expect(doc.pageCount == 1)
            #expect((doc.page(at: 0)?.string ?? "").contains("Page \(index + 1)"))
        }
    }

    @Test func rangesProduceOneFilePerRangeWithTheRightPages() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "split-ranges", text: "Page", pages: 6)

        let outputs = try PDFSplitter.split(
            input, options: PDFSplitOptions(mode: .ranges("1-3, 4-5, 6-")), to: .alongsideInput
        )

        #expect(outputs.count == 3)
        let docs = try outputs.map { try PDFDocumentIO.open($0) }
        #expect(docs.map(\.pageCount) == [3, 2, 1])

        func text(of doc: PDFDocument) -> String {
            (0..<doc.pageCount).compactMap { doc.page(at: $0)?.string }
                .joined(separator: " ")
        }

        let first = text(of: docs[0])
        #expect(first.contains("Page 1") && first.contains("Page 3"))
        #expect(!first.contains("Page 4"))

        let second = text(of: docs[1])
        #expect(second.contains("Page 4") && second.contains("Page 5"))

        let third = text(of: docs[2])
        #expect(third.contains("Page 6"))
    }

    @Test func everyNSplitsFixedChunksWithShortTail() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "split-chunks", text: "Page", pages: 7)

        let outputs = try PDFSplitter.split(
            input, options: PDFSplitOptions(mode: .everyN(3)), to: .alongsideInput
        )

        #expect(outputs.count == 3)
        let docs = try outputs.map { try PDFDocumentIO.open($0) }
        #expect(docs.map(\.pageCount) == [3, 3, 1])
    }

    @Test func rejectsChunkSizeBelowOne() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "split-zero", text: "Page")

        #expect {
            try PDFSplitter.split(
                input, options: PDFSplitOptions(mode: .everyN(0)), to: .alongsideInput
            )
        } throws: { error in
            (error as? ToolboxError)?.errorDescription?.contains("at least one page") == true
        }
    }

    @Test func rejectsRangeSelectionThatResolvesToNothing() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "split-empty", text: "Page")

        #expect {
            try PDFSplitter.split(
                input, options: PDFSplitOptions(mode: .ranges(" , ,")), to: .alongsideInput
            )
        } throws: { error in
            (error as? ToolboxError)?.errorDescription?.contains("at least one page range") == true
        }
    }

    @Test func rangeTypoSurfacesTheParserError() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "split-typo", text: "Page", pages: 2)

        #expect {
            try PDFSplitter.split(
                input, options: PDFSplitOptions(mode: .ranges("0")), to: .alongsideInput
            )
        } throws: { error in
            guard let toolboxError = error as? ToolboxError,
                  case .invalidPageRange = toolboxError else { return false }
            return true
        }
    }

    @Test func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "split-collide", text: "Page", pages: 2)

        let first = try PDFSplitter.split(
            input, options: PDFSplitOptions(mode: .everyPage), to: .alongsideInput
        )
        let second = try PDFSplitter.split(
            input, options: PDFSplitOptions(mode: .everyPage), to: .alongsideInput
        )

        #expect(first.map(\.lastPathComponent) == ["split-collide-1.pdf", "split-collide-2.pdf"])
        #expect(
            second.map(\.lastPathComponent)
                == ["split-collide-1-1.pdf", "split-collide-2-1.pdf"]
        )
    }

    @Test func writesIntoAPickedDirectory() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "split-dir", text: "Page", pages: 2)
        let folder = fixtures.directory.appendingPathComponent("chosen")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)

        let outputs = try PDFSplitter.split(
            input, options: PDFSplitOptions(mode: .everyN(1)), to: .directory(folder)
        )

        #expect(outputs.count == 2)
        // Path strings, not URLs: deletingLastPathComponent yields a URL that
        // isn't `==` to an identically-built one.
        #expect(outputs.allSatisfy { $0.deletingLastPathComponent().path == folder.path })
    }

    @Test func originalIsUntouchedAfterSplitting() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "split-intact", text: "Page", pages: 4)
        let before = try Data(contentsOf: input)

        _ = try PDFSplitter.split(
            input, options: PDFSplitOptions(mode: .ranges("1, 2-")), to: .alongsideInput
        )

        let after = try Data(contentsOf: input)
        #expect(before == after)
        let doc = try PDFDocumentIO.open(input)
        #expect(doc.pageCount == 4)
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "split-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFSplitter.split(
                locked, options: PDFSplitOptions(mode: .everyPage), to: .alongsideInput
            )
        }
    }
}
