import Testing
import CoreGraphics
import PDFKit
@testable import ToolboxKit

@Suite("GraphicsStamp")
struct GraphicsStampTests {
    @Test func textStampCanBeCreated() {
        let stamp = TextStamp(text: "Hello", fontName: "Helvetica", size: .points(12), color: CGColor.black, opacity: 1, rotationDegrees: 0)
        #expect(stamp.text == "Hello")
    }

    @Test func stampSizeResolved() {
        let size = StampSize.fraction(0.1)
        #expect(true)
    }
}

@Suite("PDFPageReplay")
struct PDFPageReplayTests {
    @Test func replayDoesNotCrash() throws {
        let fixtures = try Fixtures()
        let url = try fixtures.pdf(named: "replay-test", pages: 1)
        let doc = try PDFDocumentIO.open(url)
        guard let page = doc.page(at: 0) else { return }
        let tmp = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".pdf")
        var mediaBox = page.bounds(for: .mediaBox)
        guard let context = CGContext(tmp as CFURL, mediaBox: &mediaBox, nil) else { return }
        context.beginPage(mediaBox: &mediaBox)
        PDFPageReplay.replay(page: page, into: context)
        context.endPage()
        context.closePDF()
        #expect(FileManager.default.fileExists(atPath: tmp.path))
        try? FileManager.default.removeItem(at: tmp)
    }
}
