import Testing
import Foundation
import AppKit
import PDFKit
@testable import ToolboxKit

@Suite("PDF Signer")
struct PDFSignerTests {

    // MARK: - Geometric helpers

    /// Renders a page into a luminance grid sampled at every pixel of an
    /// exact-size bitmap (the caller passes sizes matching the page's aspect,
    /// so PDFKit doesn't letterbox). Row-major, top row first.
    ///
    /// Placement claims are verified this way rather than through text
    /// extraction, which is unreliable on CGContext-written PDFs.
    private static func renderGrid(_ page: PDFPage, width: Int = 128, height: Int = 64) -> [Double] {
        let image = page.thumbnail(of: CGSize(width: width, height: height), for: .mediaBox)
        var rect = CGRect(origin: .zero, size: CGSize(width: width, height: height))
        let cg = image.cgImage(forProposedRect: &rect, context: nil, hints: nil)!
        guard let data = cg.dataProvider?.data,
              let bytes = CFDataGetBytePtr(data),
              cg.bitsPerPixel == 32 else {
            return Array(repeating: 255, count: width * height)
        }
        let bytesPerRow = cg.bytesPerRow
        var grid = [Double]()
        grid.reserveCapacity(cg.width * cg.height)
        for y in 0..<cg.height {
            for x in 0..<cg.width {
                let offset = y * bytesPerRow + x * 4
                let b = Double(bytes[offset])
                let g = Double(bytes[offset + 1])
                let r = Double(bytes[offset + 2])
                grid.append(0.299 * r + 0.587 * g + 0.114 * b)
            }
        }
        return grid
    }

    private static func meanPixelDifference(_ a: [Double], _ b: [Double]) -> Double {
        precondition(a.count == b.count)
        return zip(a, b).map { abs($0 - $1) }.reduce(0, +) / Double(a.count)
    }

    /// Mean per-pixel difference restricted to a rectangle given in page
    /// fractions: x from the left, y from the top.
    private static func regionMeanDifference(
        _ a: [Double], _ b: [Double],
        width: Int, height: Int,
        x0: Double, x1: Double, y0: Double, y1: Double
    ) -> Double {
        precondition(a.count == b.count)
        let cols = Int(x0 * Double(width))..<max(Int(x0 * Double(width)) + 1, Int(x1 * Double(width)))
        let rows = Int(y0 * Double(height))..<max(Int(y0 * Double(height)) + 1, Int(y1 * Double(height)))
        var total = 0.0
        var count = 0
        for y in rows where y < height {
            for x in cols where x < width {
                total += abs(a[y * width + x] - b[y * width + x])
                count += 1
            }
        }
        return count == 0 ? 0 : total / Double(count)
    }

    /// Bounds of everything darker than `threshold`, in grid coordinates.
    /// On a blank white canvas this is exactly where the signature ink went.
    private static func inkBoundingBox(
        _ grid: [Double], width: Int, height: Int, darkerThan threshold: Double
    ) -> (minX: Int, maxX: Int, minY: Int, maxY: Int)? {
        var minX = width, maxX = -1, minY = height, maxY = -1
        for y in 0..<height {
            for x in 0..<width where grid[y * width + x] < threshold {
                minX = min(minX, x)
                maxX = max(maxX, x)
                minY = min(minY, y)
                maxY = max(maxY, y)
            }
        }
        guard maxX >= 0 else { return nil }
        return (minX, maxX, minY, maxY)
    }

    private static let gridWidth = 128
    private static let gridHeight = 64

    // MARK: - Tests

    @Test func typedNameLandsOnChosenPageOnly() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.blankPDF(named: "sign-range", pages: 3)

        let output = try PDFSigner.apply(
            SignOptions(content: .typedName("Alex"), anchor: .bottomRight, inset: 24, fontSize: 64),
            to: input,
            pageRangeText: "2-2",
            to: .alongsideInput
        )

        #expect(output.lastPathComponent == "sign-range-signed.pdf")
        let original = try PDFDocumentIO.open(input)
        let signed = try PDFDocumentIO.open(output)
        #expect(signed.pageCount == 3)

        // The typed name lands in the bottom-right corner (inset 24pt), so its
        // quadrant of page 2 must change clearly while everything else stays
        // put. Blank canvases make those contrasts unambiguous.
        func inkQuadrant(_ index: Int) -> Double {
            Self.regionMeanDifference(
                Self.renderGrid(original.page(at: index)!),
                Self.renderGrid(signed.page(at: index)!),
                width: Self.gridWidth, height: Self.gridHeight,
                x0: 0.55, x1: 0.98, y0: 0.45, y1: 0.95
            )
        }
        func wholePage(_ index: Int) -> Double {
            Self.meanPixelDifference(
                Self.renderGrid(original.page(at: index)!),
                Self.renderGrid(signed.page(at: index)!)
            )
        }
        #expect(inkQuadrant(1) > 15)
        #expect(wholePage(0) < 2)
        #expect(wholePage(2) < 2)
    }

    @Test func withoutARangeEveryPageIsSigned() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.blankPDF(named: "sign-all", pages: 2)

        let output = try PDFSigner.apply(
            SignOptions(content: .typedName("Alex"), anchor: .bottomRight, inset: 24, fontSize: 64),
            to: input,
            pageRangeText: nil,
            to: .alongsideInput
        )

        let original = try PDFDocumentIO.open(input)
        let signed = try PDFDocumentIO.open(output)
        for index in 0..<2 {
            let inked = Self.regionMeanDifference(
                Self.renderGrid(original.page(at: index)!),
                Self.renderGrid(signed.page(at: index)!),
                width: Self.gridWidth, height: Self.gridHeight,
                x0: 0.55, x1: 0.98, y0: 0.45, y1: 0.95
            )
            #expect(inked > 15)
        }
    }

    @Test func imageSignatureLandsAtAnchorWithAspectRatioPreserved() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.blankPDF(named: "sign-image")
        // 3:1 source. At widthFraction 0.3 of the 400pt-wide page the stamp is
        // 120×40pt, bottom-right with inset 12 → x ∈ [268, 388], y ∈ [12, 52]
        // measuring from the bottom-left origin of the page.
        let scrawl = try fixtures.signaturePNG(named: "sign-scrawl", width: 180, height: 60)

        let output = try PDFSigner.apply(
            SignOptions(content: .image(scrawl), anchor: .bottomRight, inset: 12, widthFraction: 0.3),
            to: input,
            pageRangeText: nil,
            to: .alongsideInput
        )

        let original = try PDFDocumentIO.open(input)
        let signed = try PDFDocumentIO.open(output)
        let before = Self.renderGrid(original.page(at: 0)!)
        let after = Self.renderGrid(signed.page(at: 0)!)
        let w = Self.gridWidth
        let h = Self.gridHeight

        // The destination rectangle itself must be heavily inked…
        let inked = Self.regionMeanDifference(before, after, width: w, height: h, x0: 0.67, x1: 0.97, y0: 0.74, y1: 0.94)
        #expect(inked > 40)
        // …while the rest of the page stays untouched: left half and top strip.
        let leftHalf = Self.regionMeanDifference(before, after, width: w, height: h, x0: 0, x1: 0.5, y0: 0, y1: 1)
        let topStrip = Self.regionMeanDifference(before, after, width: w, height: h, x0: 0, x1: 1, y0: 0, y1: 0.65)
        #expect(leftHalf < 2)
        #expect(topStrip < 2)

        // The full-canvas ink fixture makes the rendered bounding box expose
        // the stamp's shape exactly: dest is cols 85.8–124.2 and rows
        // 47.4–60.2 at this thumbnail scale (± a pixel of anti-aliasing).
        let bbox = Self.inkBoundingBox(after, width: w, height: h, darkerThan: 180)
        let box = try #require(bbox)
        #expect(box.minX >= 83 && box.minX <= 88)
        #expect(box.maxX >= 122 && box.maxX <= 126)
        #expect(box.minY >= 45 && box.minY <= 50)
        #expect(box.maxY >= 58 && box.maxY <= 62)

        // Its aspect must still be ~3:1 — the source was never stretched.
        let boxWidth = Double(box.maxX - box.minX + 1)
        let boxHeight = Double(box.maxY - box.minY + 1)
        #expect(boxWidth / boxHeight > 2.25)
        #expect(boxWidth / boxHeight < 3.75)
    }

    @Test func transparentSurroundOfImageStaysTransparent() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.blankPDF(named: "sign-alpha")
        // Red square covering the central half of an otherwise transparent
        // canvas. Stamped at 50% width with no inset on a 400×200 page, the
        // opaque core lands at x ∈ [250, 350], y ∈ [50, 150]; the corners of
        // the destination rectangle stay transparent in the source.
        let scrawl = try fixtures.transparentImage(named: "sign-core")

        let output = try PDFSigner.apply(
            SignOptions(content: .image(scrawl), anchor: .bottomRight, inset: 0, widthFraction: 0.5),
            to: input,
            pageRangeText: nil,
            to: .alongsideInput
        )

        let original = try PDFDocumentIO.open(input)
        let signed = try PDFDocumentIO.open(output)
        let before = Self.renderGrid(original.page(at: 0)!)
        let after = Self.renderGrid(signed.page(at: 0)!)
        let w = Self.gridWidth
        let h = Self.gridHeight

        // The opaque core shows up.
        let core = Self.regionMeanDifference(before, after, width: w, height: h, x0: 0.625, x1: 0.875, y0: 0.25, y1: 0.75)
        #expect(core > 20)
        // A corner inside the destination rectangle but outside the core must
        // be unchanged — proof the transparency survived (an opaque paste box
        // would have darkened it too).
        let deadCorner = Self.regionMeanDifference(before, after, width: w, height: h, x0: 0.95, x1: 1, y0: 0.92, y1: 1)
        #expect(deadCorner < 2)
        // Control: well outside the destination rectangle entirely.
        let farSide = Self.regionMeanDifference(before, after, width: w, height: h, x0: 0, x1: 0.4, y0: 0, y1: 1)
        #expect(farSide < 2)
    }

    @Test func signingKeepsBodyTextSelectable() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "sign-text", text: "Toolbox test document")

        let output = try PDFSigner.apply(
            SignOptions(content: .typedName("Alex"), fontSize: 36),
            to: input,
            pageRangeText: nil,
            to: .alongsideInput
        )

        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 1)
        #expect((doc.page(at: 0)?.string ?? "").contains("Toolbox test document"))
    }

    @Test func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.blankPDF(named: "sign-collide")

        let first = try PDFSigner.apply(
            SignOptions(content: .typedName("Alex")), to: input, pageRangeText: nil, to: .alongsideInput
        )
        let second = try PDFSigner.apply(
            SignOptions(content: .typedName("Alex")), to: input, pageRangeText: nil, to: .alongsideInput
        )

        #expect(first.lastPathComponent == "sign-collide-signed.pdf")
        #expect(second.lastPathComponent == "sign-collide-signed-1.pdf")
    }

    @Test func rejectsMissingInk() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.blankPDF(named: "sign-empty")

        #expect(throws: ToolboxError.emptySignature) {
            try PDFSigner.apply(
                SignOptions(content: .typedName("   ")), to: input, pageRangeText: nil, to: .alongsideInput
            )
        }

        let missing = fixtures.directory.appendingPathComponent("no-such-signature.png")
        #expect(throws: ToolboxError.decodeFailed(missing)) {
            try PDFSigner.apply(
                SignOptions(content: .image(missing)), to: input, pageRangeText: nil, to: .alongsideInput
            )
        }
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "sign-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFSigner.apply(
                SignOptions(content: .typedName("Alex")), to: locked, pageRangeText: nil, to: .alongsideInput
            )
        }
    }
}
