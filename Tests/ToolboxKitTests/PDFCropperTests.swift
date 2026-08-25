import Testing
import Foundation
import PDFKit
@testable import ToolboxKit

@Suite("PDF Cropper")
struct PDFCropperTests {
    @Test func marginCropReducesPageAndKeepsTextSelectable() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "crop-doc", text: "Crop me")

        let output = try PDFCropper.apply(
            PDFCropOptions(mode: .margins(top: 50, bottom: 50, left: 50, right: 50)),
            to: input,
            pageRangeText: nil,
            to: .alongsideInput
        )

        #expect(output.lastPathComponent == "crop-doc-cropped.pdf")
        let doc = try PDFDocumentIO.open(output)
        let box = doc.page(at: 0)!.bounds(for: .cropBox)
        #expect(abs(box.width - 300) < 0.5)
        #expect(abs(box.height - 100) < 0.5)
        #expect((doc.page(at: 0)?.string ?? "").contains("Crop me"))
    }

    @Test func regionCropAppliesOnlyToSelectedPages() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "crop-range", text: "Range crop", pages: 3)

        let output = try PDFCropper.apply(
            PDFCropOptions(mode: .region(x: 0, y: 0, width: 200, height: 200)),
            to: input,
            pageRangeText: "2-2",
            to: .alongsideInput
        )

        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 3)
        #expect(abs(doc.page(at: 0)!.bounds(for: .cropBox).width - 400) < 0.5)
        #expect(abs(doc.page(at: 1)!.bounds(for: .cropBox).width - 200) < 0.5)
        #expect(abs(doc.page(at: 2)!.bounds(for: .cropBox).width - 400) < 0.5)
    }

    @Test func clampsRegionThatExtendsPastThePage() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "crop-clamp", text: "Clamp")

        let output = try PDFCropper.apply(
            PDFCropOptions(mode: .region(x: 350, y: 150, width: 500, height: 500)),
            to: input,
            pageRangeText: nil,
            to: .alongsideInput
        )

        let doc = try PDFDocumentIO.open(output)
        let box = doc.page(at: 0)!.bounds(for: .cropBox)
        // 400×200 page, region starts at (350,150): the clamp leaves 50×50.
        #expect(abs(box.width - 50) < 0.5)
        #expect(abs(box.height - 50) < 0.5)
    }

    @Test func rejectsDegenerateMargins() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "crop-degenerate", text: "Nothing left")

        #expect {
            try PDFCropper.apply(
                PDFCropOptions(mode: .margins(top: 150, bottom: 150, left: 0, right: 0)),
                to: input,
                pageRangeText: nil,
                to: .alongsideInput
            )
        } throws: { error in
            (error as? ToolboxError)?.errorDescription?.contains("reduce the margins") == true
        }
    }

    @Test func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "crop-collide", text: "One")

        let first = try PDFCropper.apply(
            PDFCropOptions(mode: .margins(top: 10, bottom: 10, left: 10, right: 10)),
            to: input, pageRangeText: nil, to: .alongsideInput
        )
        let second = try PDFCropper.apply(
            PDFCropOptions(mode: .margins(top: 10, bottom: 10, left: 10, right: 10)),
            to: input, pageRangeText: nil, to: .alongsideInput
        )

        #expect(first.lastPathComponent == "crop-collide-cropped.pdf")
        #expect(second.lastPathComponent == "crop-collide-cropped-1.pdf")
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "crop-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFCropper.apply(
                PDFCropOptions(mode: .margins(top: 1, bottom: 1, left: 1, right: 1)),
                to: locked, pageRangeText: nil, to: .alongsideInput
            )
        }
    }
}
