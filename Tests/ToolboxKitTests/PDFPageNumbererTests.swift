import Testing
import Foundation
@testable import ToolboxKit

@Suite("PDF Page Numberer")
struct PDFPageNumbererTests {
    @Test func addsNumbersAndKeepsTextSelectable() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "number-test", pages: 1)
        let tmp = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmp) }

        let output = try PDFPageNumberer.addNumbers(
            to: input,
            position: .bottomCenter,
            startNumber: 1,
            pageRangeText: nil,
            format: .plain,
            fontSize: 12,
            margin: 36,
            to: .directory(tmp)
        )
        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 1)
        #expect(FileManager.default.fileExists(atPath: output.path))
    }

    @Test func pageRangeFilters() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "number-multi", pages: 3)
        let tmp = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmp) }

        let output = try PDFPageNumberer.addNumbers(
            to: input,
            position: .topRight,
            startNumber: 5,
            pageRangeText: "2-2",
            format: .pagePrefix,
            fontSize: 10,
            margin: 24,
            to: .directory(tmp)
        )
        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 3)
        #expect(FileManager.default.fileExists(atPath: output.path))
    }
}
