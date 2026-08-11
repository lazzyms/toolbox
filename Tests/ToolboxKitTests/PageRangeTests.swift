import Foundation
import PDFKit
import Testing
@testable import ToolboxKit

@Suite("PageRange")
struct PageRangeTests {

    @Test("a single page is converted to 0-based")
    func singlePage() throws {
        #expect(try PageRange.parse("7", pageCount: 10) == [6])
    }

    @Test("several singles are sorted and deduplicated")
    func multipleSingles() throws {
        #expect(try PageRange.parse("2,4,6", pageCount: 10) == [1, 3, 5])
        #expect(try PageRange.parse("6,4,2", pageCount: 10) == [1, 3, 5])
        #expect(try PageRange.parse("2,2,2", pageCount: 10) == [1])
    }

    @Test("a range is inclusive and 0-based")
    func range() throws {
        #expect(try PageRange.parse("1-3", pageCount: 10) == [0, 1, 2])
    }

    @Test("the issue's example parses")
    func example() throws {
        #expect(try PageRange.parse("1-3, 7, 9-", pageCount: 10) == [0, 1, 2, 6, 8, 9])
    }

    @Test("open-ended ranges reach the document edge")
    func openEnded() throws {
        #expect(try PageRange.parse("9-", pageCount: 10) == [8, 9])
        #expect(try PageRange.parse("-3", pageCount: 10) == [0, 1, 2])
        #expect(try PageRange.parse("1-", pageCount: 10) == [0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
    }

    @Test("reversed input is the range between the two endpoints")
    func reversed() throws {
        #expect(try PageRange.parse("7-3", pageCount: 10) == [2, 3, 4, 5, 6])
    }

    @Test("overlapping ranges collapse")
    func overlapsCollapse() throws {
        #expect(try PageRange.parse("1-5, 3-7", pageCount: 10) == [0, 1, 2, 3, 4, 5, 6])
    }

    @Test("odd and even select every other page, case-insensitive")
    func oddEven() throws {
        #expect(try PageRange.parse("odd", pageCount: 6) == [0, 2, 4])
        #expect(try PageRange.parse("EVEN", pageCount: 6) == [1, 3, 5])
        #expect(try PageRange.parse("Odd, 2", pageCount: 6) == [0, 1, 2, 4])
    }

    @Test("empty input means every page")
    func emptyMeansAll() throws {
        #expect(try PageRange.parse("", pageCount: 5) == [0, 1, 2, 3, 4])
        #expect(try PageRange.parse("  ", pageCount: 5) == [0, 1, 2, 3, 4])
    }

    @Test("zero is rejected")
    func zeroRejected() throws {
        #expect(throws: ToolboxError.self) {
            _ = try PageRange.parse("0", pageCount: 10)
        }
        #expect(throws: ToolboxError.self) {
            _ = try PageRange.parse("0-5", pageCount: 10)
        }
    }

    @Test("out-of-bounds pages throw instead of clamping")
    func outOfBoundsThrows() throws {
        #expect(throws: ToolboxError.self) {
            _ = try PageRange.parse("1-999", pageCount: 10)
        }
        #expect(throws: ToolboxError.self) {
            _ = try PageRange.parse("9-", pageCount: 8)
        }
        #expect(throws: ToolboxError.self) {
            _ = try PageRange.parse("11", pageCount: 10)
        }
    }

    @Test("malformed tokens throw")
    func malformedThrows() throws {
        for bad in ["abc", "1a", "1-3-5", "--", "-", "1-2-3-4"] {
            #expect(throws: ToolboxError.self) {
                _ = try PageRange.parse(bad, pageCount: 10)
            }
        }
    }

    @Test("whitespace around separators is tolerated")
    func spacesTolerated() throws {
        #expect(try PageRange.parse("1 - 3", pageCount: 10) == [0, 1, 2])
        #expect(try PageRange.parse("1- 3, 7", pageCount: 10) == [0, 1, 2, 6])
    }

    @Test("an empty document has nothing to select")
    func emptyDocument() throws {
        #expect(try PageRange.parse("", pageCount: 0) == [])
        #expect(throws: ToolboxError.self) {
            _ = try PageRange.parse("1", pageCount: 0)
        }
    }

    @Test("the error names the offending page")
    func errorMessageNamesPage() throws {
        do {
            _ = try PageRange.parse("1-999", pageCount: 10)
            Issue.record("expected an error")
        } catch let error as ToolboxError {
            guard case .invalidPageRange(let reason) = error else {
                Issue.record("wrong error case")
                return
            }
            #expect(reason.contains("999"))
        }
    }
}

@Suite("PDFDocumentIO")
struct PDFDocumentIOTests {

    @Test("open returns a plain document")
    func opensPlain() throws {
        let fixtures = try Fixtures()
        let url = try fixtures.pdf(named: "plain")

        let doc = try PDFDocumentIO.open(url)

        #expect(doc.pageCount == 1)
        #expect(!doc.isLocked)
    }

    @Test("open throws notAPDF for unreadable input")
    func openRejectsCorrupt() throws {
        let fixtures = try Fixtures()
        let corrupt = fixtures.directory.appendingPathComponent("bad.pdf")
        try Data("%PDF-1.4 truncated".utf8).write(to: corrupt)

        #expect(throws: ToolboxError.notAPDF(corrupt)) {
            _ = try PDFDocumentIO.open(corrupt)
        }
    }

    @Test("open rejects non-PDF extensions")
    func openRejectsNonPDF() throws {
        let fixtures = try Fixtures()
        let text = fixtures.directory.appendingPathComponent("note.txt")
        try "not a pdf".write(to: text, atomically: true, encoding: .utf8)

        #expect(throws: ToolboxError.unsupportedInput("txt")) {
            _ = try PDFDocumentIO.open(text)
        }
    }

    @Test("unlock needs the right password")
    func unlockWithPassword() throws {
        let fixtures = try Fixtures()
        let url = try fixtures.pdf(named: "locked", password: "s3cret")
        let doc = try PDFDocumentIO.open(url)
        #expect(doc.isLocked)

        #expect(throws: ToolboxError.wrongPassword) {
            try PDFDocumentIO.unlock(doc, password: "guess")
        }
        try PDFDocumentIO.unlock(doc, password: "s3cret")
        #expect(!doc.isLocked)
    }

    @Test("copy rebuilds into a fresh document with no encryption")
    func copyDropsEncryption() throws {
        let fixtures = try Fixtures()
        let url = try fixtures.pdf(named: "locked", password: "s3cret", pages: 3)
        let doc = try PDFDocumentIO.open(url)
        try PDFDocumentIO.unlock(doc, password: "s3cret")

        let rebuilt = PDFDocumentIO.copy(doc)
        #expect(rebuilt.pageCount == 3)

        let output = OutputNaming.destination(
            for: url, in: .directory(fixtures.directory), suffix: "-copy", extension: "pdf"
        )
        try PDFDocumentIO.save(rebuilt, to: output)

        let check = try PDFDocumentIO.open(output)
        #expect(!check.isLocked)
        #expect(!check.isEncrypted)
        #expect(check.pageCount == 3)
    }

    @Test("save throws writeFailed instead of returning false")
    func saveThrows() throws {
        let fixtures = try Fixtures()
        let source = try PDFDocumentIO.open(try fixtures.pdf(named: "page"))
        let blocker = fixtures.directory.appendingPathComponent("blocker")
        try Data("x".utf8).write(to: blocker)
        let impossible = blocker.appendingPathComponent("out.pdf")

        #expect(throws: ToolboxError.writeFailed(impossible)) {
            try PDFDocumentIO.save(PDFDocumentIO.copy(source), to: impossible)
        }
    }
}
