import Testing
import Foundation
import PDFKit
@testable import ToolboxKit

@Suite("Images to PDF")
struct ImagesToPDFTests {
    @Test func onePagePerImageInQueueOrder() throws {
        let fixtures = try Fixtures()
        // Distinct shapes make the order readable from the page boxes alone:
        // a 3:1 landscape lands first, a 1:3 portrait second.
        let wide = try fixtures.image(named: "order-wide", width: 60, height: 20)
        let tall = try fixtures.image(named: "order-tall", width: 20, height: 60)

        let output = try ImagesToPDF.build(
            [wide, tall],
            options: ImagesToPDFOptions(pageSize: .fitToImage),
            to: .alongsideInput
        )

        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 2)
        let first = doc.page(at: 0)!.bounds(for: .mediaBox)
        let second = doc.page(at: 1)!.bounds(for: .mediaBox)
        #expect(first.width > first.height)
        #expect(second.height > second.width)
    }

    @Test func bakesEXIFOrientationIntoThePage() throws {
        let fixtures = try Fixtures()
        // Orientation 6 means "rotate a quarter turn clockwise to display", so
        // the 40×20 pixels show as 20×40. The page must agree with the display.
        let input = try fixtures.orientedJPEG(named: "exif-photo", orientation: 6)

        let output = try ImagesToPDF.build(
            [input],
            options: ImagesToPDFOptions(pageSize: .fitToImage),
            to: .alongsideInput
        )

        let doc = try PDFDocumentIO.open(output)
        let box = doc.page(at: 0)!.bounds(for: .mediaBox)
        #expect(abs(box.width - 20) < 1.5)
        #expect(abs(box.height - 40) < 1.5)
    }

    @Test func fixedPageSizeWithMargins() throws {
        let fixtures = try Fixtures()
        let image = try fixtures.threeToneImage(named: "fixed-tone", width: 60, height: 20)

        let output = try ImagesToPDF.build(
            [image],
            options: ImagesToPDFOptions(pageSize: .a4, orientation: .portrait, margin: 36),
            to: .alongsideInput
        )

        let doc = try PDFDocumentIO.open(output)
        let box = doc.page(at: 0)!.bounds(for: .mediaBox)
        #expect(abs(box.width - 595) < 0.5)
        #expect(abs(box.height - 842) < 0.5)
    }

    @Test func fittedRectPreservesAspectAndNeverUpscales() throws {
        let page = CGRect(x: 0, y: 0, width: 595, height: 842)
        let margin = 36.0

        // A huge source is capped at its natural size (scale ≤ 1)…
        let huge = ImagesToPDF.fittedRect(
            imageSize: CGSize(width: 3000, height: 1500), inPage: page, margin: margin
        )
        #expect(huge.width == 3000 - 0 || huge.width <= 3000)
        #expect(huge.width < page.width - margin * 2 + 1)
        #expect(abs((huge.width / huge.height) - 2) < 0.01)

        // …and a tiny source is never blown up.
        let tiny = ImagesToPDF.fittedRect(
            imageSize: CGSize(width: 30, height: 10), inPage: page, margin: margin
        )
        #expect(tiny.width == 30)
        #expect(tiny.height == 10)
    }

    @Test func landscapeSwapsTheFixedPage() throws {
        let fixtures = try Fixtures()
        let image = try fixtures.image(named: "land-photo", width: 40, height: 30)

        let output = try ImagesToPDF.build(
            [image],
            options: ImagesToPDFOptions(pageSize: .usLetter, orientation: .landscape),
            to: .alongsideInput
        )

        let doc = try PDFDocumentIO.open(output)
        let box = doc.page(at: 0)!.bounds(for: .mediaBox)
        #expect(abs(box.width - 792) < 0.5)
        #expect(abs(box.height - 612) < 0.5)
    }

    @Test func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let image = try fixtures.image(named: "collide-img", width: 40, height: 20)

        let first = try ImagesToPDF.build([image], options: ImagesToPDFOptions(), to: .alongsideInput)
        let second = try ImagesToPDF.build([image], options: ImagesToPDFOptions(), to: .alongsideInput)

        #expect(first.lastPathComponent == "collide-img.pdf")
        #expect(second.lastPathComponent == "collide-img-1.pdf")
    }

    @Test func rejectsEmptySelection() throws {
        #expect(throws: ToolboxError.emptySelection) {
            try ImagesToPDF.build([], options: ImagesToPDFOptions(), to: .alongsideInput)
        }
    }
}
