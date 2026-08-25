import Testing
import Foundation
import UniformTypeIdentifiers
@testable import ToolboxKit

@Suite("PDF Image Exporter")
struct PDFImageExporterTests {
    @Test func rendersEveryPageAtTheRequestedDPI() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "render-doc", text: "Render me", pages: 2)

        let outputs = try PDFImageExporter.convert(
            input,
            options: PDFToImagesOptions(format: .png, dpi: 150),
            pageRangeText: nil,
            to: .alongsideInput
        )

        #expect(outputs.count == 2)
        // 400×200 pt at 150 DPI = 833×417 px.
        let size = try #require(Fixtures.pixelSize(of: outputs[0]))
        #expect(abs(size.width - 833) < 3)
        #expect(abs(size.height - 417) < 3)
    }

    @Test func namesCarryTheDocumentPageNumber() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "naming-doc", text: "Pages", pages: 3)

        let outputs = try PDFImageExporter.convert(
            input,
            options: PDFToImagesOptions(format: .png, dpi: 72),
            pageRangeText: "2-2",
            to: .alongsideInput
        )

        #expect(outputs.count == 1)
        #expect(outputs[0].lastPathComponent == "naming-doc-page-2.png")
    }

    @Test func jpegOutputEncodesAsJPEG() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "jpeg-doc", text: "Jpeg body")

        let outputs = try PDFImageExporter.convert(
            input,
            options: PDFToImagesOptions(format: .jpeg, quality: 0.8, dpi: 72),
            pageRangeText: nil,
            to: .alongsideInput
        )

        #expect(outputs.count == 1)
        #expect(Fixtures.format(of: outputs[0]) == UTType.jpeg.identifier)
    }

    @Test func capsAbsurdResolutions() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "huge-doc", text: "Poster")

        #expect(throws: ToolboxError.resolutionTooLarge(33333, 16667)) {
            try PDFImageExporter.convert(
                input,
                options: PDFToImagesOptions(format: .png, dpi: 6000),
                pageRangeText: nil,
                to: .alongsideInput
            )
        }
    }

    @Test func rerunCollidesAreNumberedNotOverwritten() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "collide-render", text: "Twice")

        let first = try PDFImageExporter.convert(
            input, options: PDFToImagesOptions(format: .png, dpi: 72), pageRangeText: nil, to: .alongsideInput
        )
        let second = try PDFImageExporter.convert(
            input, options: PDFToImagesOptions(format: .png, dpi: 72), pageRangeText: nil, to: .alongsideInput
        )

        #expect(first[0].lastPathComponent == "collide-render-page-1.png")
        #expect(second[0].lastPathComponent.contains("-1"))
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "render-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFImageExporter.convert(
                locked, options: PDFToImagesOptions(format: .png), pageRangeText: nil, to: .alongsideInput
            )
        }
    }
}
