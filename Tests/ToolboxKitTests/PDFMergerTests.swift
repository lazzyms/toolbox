import Testing
import Foundation
import PDFKit
@testable import ToolboxKit

@Suite("PDF Merger")
struct PDFMergerTests {
    @Test func mergesPagesInOrderAndKeepsTextSelectable() throws {
        let fixtures = try Fixtures()
        let first = try fixtures.pdf(named: "merge-first", text: "Alpha document")
        let second = try fixtures.pdf(named: "merge-second", text: "Beta document")

        let output = try PDFMerger.merge([first, second], to: .alongsideInput)

        #expect(output.lastPathComponent == "merge-first-merged.pdf")
        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 2)
        #expect(doc.page(at: 0)?.string?.contains("Alpha") == true)
        #expect(doc.page(at: 1)?.string?.contains("Beta") == true)

        // Originals are untouched and still open on their own.
        #expect(FileManager.default.fileExists(atPath: first.path))
        let firstAgain = try PDFDocumentIO.open(first)
        #expect(firstAgain.pageCount == 1)
    }

    @Test func multiPageInputsKeepEveryPage() throws {
        let fixtures = try Fixtures()
        let single = try fixtures.pdf(named: "merge-single", pages: 1)
        let triple = try fixtures.pdf(named: "merge-triple", pages: 3)

        let output = try PDFMerger.merge([single, triple], to: .alongsideInput)

        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 4)
    }

    @Test func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let first = try fixtures.pdf(named: "collide", text: "One")
        let second = try fixtures.pdf(named: "collide-b", text: "Two")

        let firstOutput = try PDFMerger.merge([first, second], to: .alongsideInput)
        let secondOutput = try PDFMerger.merge([first, second], to: .alongsideInput)

        #expect(firstOutput.lastPathComponent == "collide-merged.pdf")
        #expect(secondOutput.lastPathComponent == "collide-merged-1.pdf")
        #expect(FileManager.default.fileExists(atPath: firstOutput.path))
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "locked", password: "s3cret")
        let open = try fixtures.pdf(named: "open-doc", text: "Open")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFMerger.merge([open, locked], to: .alongsideInput)
        }
    }

    @Test func rejectsEmptyInput() throws {
        #expect(throws: ToolboxError.self) {
            try PDFMerger.merge([], to: .alongsideInput)
        }
    }

    @Test func directoryLocationWritesThere() throws {
        let fixtures = try Fixtures()
        let outDir = fixtures.directory.appendingPathComponent("out")
        try FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)
        let first = try fixtures.pdf(named: "dir-merge-a", text: "A")
        let second = try fixtures.pdf(named: "dir-merge-b", text: "B")

        let output = try PDFMerger.merge([first, second], to: .directory(outDir))

        #expect(output.deletingLastPathComponent().path == outDir.path)
        #expect(FileManager.default.fileExists(atPath: output.path))
    }
}
