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

    @Test func drawTextDoesNotCrash() {
        var data = Data()
        guard let consumer = CGDataConsumer(data: &data) else { return }
        var rect = CGRect(x: 0, y: 0, width: 200, height: 200)
        guard let context = CGContext(consumer: consumer, mediaBox: &rect, nil) else { return }
        let stamp = TextStamp(text: "Test", size: .points(12))
        GraphicsStamp.draw(stamp, in: context, bounds: rect, anchor: .center)
    }

    @Test func drawImageDoesNotCrash() {
        var data = Data()
        guard let consumer = CGDataConsumer(data: &data) else { return }
        var rect = CGRect(x: 0, y: 0, width: 200, height: 200)
        guard let context = CGContext(consumer: consumer, mediaBox: &rect, nil) else { return }
        let size = CGSize(width: 10, height: 10)
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        let bitmapInfo = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue)
        guard let ctx = CGContext(data: nil, width: Int(size.width), height: Int(size.height), bitsPerComponent: 8, bytesPerRow: 0, space: colorSpace, bitmapInfo: bitmapInfo.rawValue) else { return }
        ctx.setFillColor(CGColor.black)
        ctx.fill(CGRect(origin: .zero, size: size))
        guard let cgImage = ctx.makeImage() else { return }
        GraphicsStamp.draw(cgImage, in: context, bounds: rect, anchor: .center, scale: 1, opacity: 1)
    }
}

@Suite("PDFPageReplay")
struct PDFPageReplayTests {
    @Test func replayDoesNotCrash() throws {
        let url = Fixtures.pdfSimple
        let doc = try PDFDocumentIO.open(url)
        guard let page = doc.page(at: 0) else { return }
        var data = Data()
        guard let consumer = CGDataConsumer(data: &data) else { return }
        let rect = page.bounds(for: .mediaBox)
        var mediaBox = rect
        guard let context = CGContext(consumer: consumer, mediaBox: &mediaBox, nil) else { return }
        context.beginPage(mediaBox: &mediaBox)
        PDFPageReplay.replay(page: page, into: context)
        context.endPage()
    }
}
