import Testing
import Foundation
import AppKit
import PDFKit
@testable import ToolboxKit

@Suite("PDF Watermarker")
struct PDFWatermarkerTests {
    /// Renders a page to a coarse luminance grid so two renders can be
    /// compared numerically without depending on text extraction.
    static func luminanceGrid(_ page: PDFPage, side: Int = 48) -> [Double] {
        let image = page.thumbnail(of: CGSize(width: side, height: side), for: .mediaBox)
        var rect = CGRect(origin: .zero, size: CGSize(width: side, height: side))
        let cg = image.cgImage(forProposedRect: &rect, context: nil, hints: nil)!
        guard let data = cg.dataProvider?.data,
              let bytes = CFDataGetBytePtr(data),
              cg.bitsPerPixel == 32 else {
            return Array(repeating: 255, count: side * side)
        }
        let w = cg.width
        let h = cg.height
        let bytesPerRow = cg.bytesPerRow
        var grid = [Double]()
        grid.reserveCapacity(w * h)
        for y in stride(from: 0, to: h, by: max(1, h / side)) {
            for x in stride(from: 0, to: w, by: max(1, w / side)) {
                let offset = y * bytesPerRow + x * 4
                let b = Double(bytes[offset])
                let g = Double(bytes[offset + 1])
                let r = Double(bytes[offset + 2])
                grid.append(0.299 * r + 0.587 * g + 0.114 * b)
            }
        }
        return grid
    }

    static func meanPixelDifference(_ a: [Double], _ b: [Double]) -> Double {
        precondition(a.count == b.count)
        return zip(a, b).map { abs($0 - $1) }.reduce(0, +) / Double(a.count)
    }
    @Test func textWatermarkKeepsOriginalTextSelectable() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "wm-doc", text: "Toolbox test document")

        let output = try PDFWatermarker.apply(
            WatermarkOptions(content: .text("DRAFT"), opacity: 0.4),
            to: input,
            pageRangeText: nil,
            to: .alongsideInput
        )

        #expect(output.lastPathComponent == "wm-doc-watermarked.pdf")
        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 1)
        let pageText = doc.page(at: 0)?.string ?? ""
        #expect(pageText.contains("Toolbox test document"))
        #expect(pageText.contains("DRAFT"))
    }

    @Test func imageWatermarkWritesWithoutDistortionPath() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "wm-image", text: "Body")
        let logo = try fixtures.threeToneImage(named: "wm-logo", width: 90, height: 30)

        let output = try PDFWatermarker.apply(
            WatermarkOptions(content: .image(logo), imageScale: 0.5),
            to: input,
            pageRangeText: nil,
            to: .alongsideInput
        )

        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 1)
        #expect((doc.page(at: 0)?.string ?? "").contains("Body"))
    }

    @Test func pageRangeLimitsWhereStampsLand() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "wm-range", text: "Range page", pages: 3)

        let output = try PDFWatermarker.apply(
            WatermarkOptions(content: .text("SECRET"), fontSize: 80, color: .black, opacity: 0.95),
            to: input,
            pageRangeText: "2-2",
            to: .alongsideInput
        )

        // Text extraction is unreliable on CGContext-written PDFs (text can
        // bleed between pages), so verify geometrically instead: the stamped
        // page must differ visually from the original, the others must not.
        let original = try PDFDocumentIO.open(input)
        let watermarked = try PDFDocumentIO.open(output)

        let untouched = Self.meanPixelDifference(
            Self.luminanceGrid(original.page(at: 0)!),
            Self.luminanceGrid(watermarked.page(at: 0)!)
        )
        let stamped = Self.meanPixelDifference(
            Self.luminanceGrid(original.page(at: 1)!),
            Self.luminanceGrid(watermarked.page(at: 1)!)
        )
        // Relational so the thresholds survive renderer differences across
        // macOS versions: the stamped page must stand out clearly, and the
        // untouched pages must stay close to the original render.
        #expect(stamped > untouched * 1.8)
        #expect(stamped > 8)
        #expect(untouched < 7)
    }

    @Test func tiledAnchorStampsRepeatedly() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "wm-tiled", text: "Once?")

        let output = try PDFWatermarker.apply(
            WatermarkOptions(content: .text("TILE"), fontSize: 12, anchor: .tiled(spacing: 40)),
            to: input,
            pageRangeText: nil,
            to: .alongsideInput
        )

        let doc = try PDFDocumentIO.open(output)
        let pageText = doc.page(at: 0)?.string ?? ""
        let count = pageText.components(separatedBy: "TILE").count - 1
        #expect(count > 1)
    }

    @Test func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "wm-collide", text: "One")

        let first = try PDFWatermarker.apply(
            WatermarkOptions(content: .text("DRAFT")), to: input, pageRangeText: nil, to: .alongsideInput
        )
        let second = try PDFWatermarker.apply(
            WatermarkOptions(content: .text("DRAFT")), to: input, pageRangeText: nil, to: .alongsideInput
        )

        #expect(first.lastPathComponent == "wm-collide-watermarked.pdf")
        #expect(second.lastPathComponent == "wm-collide-watermarked-1.pdf")
    }

    @Test func rejectsEmptyText() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "wm-empty", text: "Body")

        #expect(throws: ToolboxError.emptyWatermark) {
            try PDFWatermarker.apply(
                WatermarkOptions(content: .text("   ")), to: input, pageRangeText: nil, to: .alongsideInput
            )
        }
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "wm-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFWatermarker.apply(
                WatermarkOptions(content: .text("DRAFT")), to: locked, pageRangeText: nil, to: .alongsideInput
            )
        }
    }
}
