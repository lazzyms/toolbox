import Testing
import Foundation
import Compression
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
@testable import ToolboxKit

@Suite("PDF Image Extractor")
struct PDFImageExtractorTests {

    // MARK: - Quartz-produced PDFs

    @Test func extractsTheEmbeddedRasterFromAnImageOnlyPDF() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.imageOnlyPDF(named: "scan-doc")

        let result = try PDFImageExtractor.extract(
            input, options: PDFExtractImagesOptions(minSize: 1), pageRangeText: nil, to: .alongsideInput
        )

        #expect(result.outputs.count == 1)
        #expect(result.unsupported == 0)
        let output = try #require(result.outputs.first)
        #expect(output.pathExtension == "png")
        let size = try #require(Fixtures.pixelSize(of: output))
        #expect(size.width == 100)
        #expect(size.height == 50)
    }

    @Test func rerunCollidesAreNumberedNotOverwritten() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.imageOnlyPDF(named: "rerun-doc")
        let options = PDFExtractImagesOptions()

        let first = try PDFImageExtractor.extract(input, options: options, pageRangeText: nil, to: .alongsideInput)
        let second = try PDFImageExtractor.extract(input, options: options, pageRangeText: nil, to: .alongsideInput)

        #expect(first.outputs.map(\.lastPathComponent) == ["rerun-doc-p1-1.png"])
        #expect(second.outputs.count == first.outputs.count)
        _ = try #require(second.outputs.first)
        #expect(Set(second.outputs.map(\.lastPathComponent)).isDisjoint(with: first.outputs.map(\.lastPathComponent)))
        #expect(second.outputs.allSatisfy { $0.lastPathComponent.contains("-1.") })
    }

    @Test func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "extract-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFImageExtractor.extract(
                locked, options: PDFExtractImagesOptions(), pageRangeText: nil, to: .alongsideInput
            )
        }
    }

    // MARK: - DCTDecode passthrough

    @Test func jpegComesOutByteIdenticalNotReencoded() throws {
        let fixtures = try Fixtures()
        try #require(ImageFormat.jpeg.canEncode)
        let photo = try fixtures.image(named: "photo", width: 64, height: 48, format: .jpeg)
        let jpegBytes = try Data(contentsOf: photo)

        let input = try fixtures.rawPDF(named: "dct-doc", objects: [
            RawObject.catalog,
            RawObject.pages(kids: [3]),
            RawObject.page(id: 3, contents: 4, xobjects: "/Im0 5 0 R"),
            RawObject.content(id: 4, ops: "q 100 0 0 100 0 0 cm /Im0 Do Q"),
            (
                5,
                "<< /Type /XObject /Subtype /Image /Width 64 /Height 48"
                    + " /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length __LENGTH__ >>",
                jpegBytes
            ),
        ])

        let result = try PDFImageExtractor.extract(
            input, options: PDFExtractImagesOptions(), pageRangeText: nil, to: .alongsideInput
        )

        #expect(result.outputs.map(\.lastPathComponent) == ["dct-doc-p1-1.jpg"])
        let written = try Data(contentsOf: try #require(result.outputs.first))
        #expect(written == jpegBytes)
        #expect(Fixtures.format(of: try #require(result.outputs.first)) == UTType.jpeg.identifier)
    }

    // MARK: - FlateDecode sample reconstruction

    @Test func rawSamplesRebuildIntoPixelAccuratePNG() throws {
        try assertPixelsSurvive(named: "raw-doc", filterHead: "", payload: Self.twoByTwoRGB())
    }

    @Test func wrappedFlateStreamDecodesToSamePixels() throws {
        try assertPixelsSurvive(
            named: "wrapped-doc",
            filterHead: " /Filter /FlateDecode",
            payload: zlibWrapped(Self.twoByTwoRGB())
        )
    }

    @Test func pngPredictorRowsAreUnfilteredCorrectly() throws {
        // Row 1 carries type 0 (None), row 2 type 2 (Up): the deltas against
        // row 1 are what a viewer reverses before painting.
        let trueRows = Self.twoByTwoRGB()
        var predicted = Data([0x00])
        predicted.append(trueRows[0..<6])
        predicted.append(contentsOf: [0x02])
        for (index, byte) in trueRows[6..<12].enumerated() {
            predicted.append(byte &- trueRows[index])
        }

        let fixtures = try Fixtures()
        let input = try fixtures.rawPDF(named: "predictor-doc", objects: [
            RawObject.catalog,
            RawObject.pages(kids: [3]),
            RawObject.page(id: 3, contents: 4, xobjects: "/Im0 5 0 R"),
            RawObject.content(id: 4, ops: "q 20 0 0 20 0 0 cm /Im0 Do Q"),
            (
                5,
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 2"
                    + " /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode"
                    + " /DecodeParms << /Predictor 15 /Colors 3 /BitsPerComponent 8 /Columns 2 >>"
                    + " /Length __LENGTH__ >>",
                zlibWrapped(predicted)
            ),
        ])

        let result = try PDFImageExtractor.extract(
            input, options: PDFExtractImagesOptions(minSize: 1), pageRangeText: nil, to: .alongsideInput
        )

        #expect(result.outputs.count == 1)
        try assertRedGreenBlueWhite(try #require(result.outputs.first))
    }

    private func assertPixelsSurvive(named name: String, filterHead: String, payload: Data) throws {
        let fixtures = try Fixtures()
        let input = try fixtures.rawPDF(named: name, objects: [
            RawObject.catalog,
            RawObject.pages(kids: [3]),
            RawObject.page(id: 3, contents: 4, xobjects: "/Im0 5 0 R"),
            RawObject.content(id: 4, ops: "q 20 0 0 20 0 0 cm /Im0 Do Q"),
            (
                5,
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 2"
                    + " /ColorSpace /DeviceRGB /BitsPerComponent 8\(filterHead) /Length __LENGTH__ >>",
                payload
            ),
        ])

        let result = try PDFImageExtractor.extract(
            input, options: PDFExtractImagesOptions(minSize: 1), pageRangeText: nil, to: .alongsideInput
        )

        #expect(result.outputs.count == 1)
        try assertRedGreenBlueWhite(try #require(result.outputs.first))
    }

    /// Red, green in the first raster row; blue, white in the second —
    /// distinct channels make a swapped or shifted reconstruction unmistakable.
    private static func twoByTwoRGB() -> Data {
        Data([0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF])
    }

    private func assertRedGreenBlueWhite(_ url: URL) throws {
        let size = try #require(Fixtures.pixelSize(of: url))
        #expect(size.width == 2)
        #expect(size.height == 2)
        // Sample row 0 is the first raster line; drawn into a context it lands
        // at y = 0, so red/green sit at the bottom of the rendered output.
        let left0 = try pixel(at: 0, 0, in: url)
        #expect(left0.r == 255 && left0.g == 0 && left0.b == 0)
        let right0 = try pixel(at: 1, 0, in: url)
        #expect(right0.g == 255 && right0.r == 0 && right0.b == 0)
        let left1 = try pixel(at: 0, 1, in: url)
        #expect(left1.b == 255 && left1.r == 0 && left1.g == 0)
        let right1 = try pixel(at: 1, 1, in: url)
        #expect(right1.r == 255 && right1.g == 255 && right1.b == 255)
    }

    /// Reads back an output through an sRGB canvas, y measured from the bottom.
    private func pixel(at x: Int, _ y: Int, in url: URL) throws -> (r: UInt8, g: UInt8, b: UInt8, a: UInt8) {
        let source = try #require(CGImageSourceCreateWithURL(url as CFURL, nil))
        let image = try #require(CGImageSourceCreateImageAtIndex(source, 0, nil))
        let context = try #require(CGContext(
            data: nil, width: image.width, height: image.height,
            bitsPerComponent: 8, bytesPerRow: image.width * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ))
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        let buffer = try #require(context.data)
        let offset = (y * image.width + x) * 4
        let bytes = buffer.assumingMemoryBound(to: UInt8.self)
        return (bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3])
    }

    @Test func ascii85WrappedGrayStreamDecodes() throws {
        // "z" expands to four zero bytes: a fully black 4×1 grayscale image.
        let fixtures = try Fixtures()
        let input = try fixtures.rawPDF(named: "a85-doc", objects: [
            RawObject.catalog,
            RawObject.pages(kids: [3]),
            RawObject.page(id: 3, contents: 4, xobjects: "/Im0 5 0 R"),
            RawObject.content(id: 4, ops: "q 40 0 0 10 0 0 cm /Im0 Do Q"),
            (
                5,
                "<< /Type /XObject /Subtype /Image /Width 4 /Height 1"
                    + " /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /ASCII85Decode /Length __LENGTH__ >>",
                Data("z~>".utf8)
            ),
        ])

        let result = try PDFImageExtractor.extract(
            input, options: PDFExtractImagesOptions(minSize: 1), pageRangeText: nil, to: .alongsideInput
        )

        #expect(result.outputs.count == 1)
        #expect(result.unsupported == 0)
        let output = try #require(result.outputs.first)
        let size = try #require(Fixtures.pixelSize(of: output))
        #expect(size.width == 4)
        #expect(size.height == 1)
        let shade = try pixel(at: 2, 0, in: output)
        #expect(shade.r == 0 && shade.a == 255)
    }

    // MARK: - Honest skipping

    @Test func unsupportedCodecIsCountedAndNeverWritten() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.rawPDF(named: "jpx-doc", objects: [
            RawObject.catalog,
            RawObject.pages(kids: [3]),
            RawObject.page(id: 3, contents: 4, xobjects: "/Im0 5 0 R"),
            RawObject.content(id: 4, ops: "q 50 0 0 50 0 0 cm /Im0 Do Q"),
            (
                5,
                "<< /Type /XObject /Subtype /Image /Width 16 /Height 16"
                    + " /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /JPXDecode /Length __LENGTH__ >>",
                Data(repeating: 0xAB, count: 64)
            ),
        ])

        let result = try PDFImageExtractor.extract(
            input, options: PDFExtractImagesOptions(minSize: 1), pageRangeText: nil, to: .alongsideInput
        )

        #expect(result.outputs.isEmpty)
        #expect(result.unsupported == 1)
        let written = try FileManager.default.contentsOfDirectory(atPath: input.deletingLastPathComponent().path)
        #expect(written.filter { $0.contains("-p") }.isEmpty)
    }

    @Test func corruptJPEGPayloadCountsAsUnsupported() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.rawPDF(named: "fakedct-doc", objects: [
            RawObject.catalog,
            RawObject.pages(kids: [3]),
            RawObject.page(id: 3, contents: 4, xobjects: "/Im0 5 0 R"),
            RawObject.content(id: 4, ops: "q 50 0 0 50 0 0 cm /Im0 Do Q"),
            (
                5,
                "<< /Type /XObject /Subtype /Image /Width 16 /Height 16"
                    + " /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length __LENGTH__ >>",
                Data("this is not a jpeg".utf8)
            ),
        ])

        let result = try PDFImageExtractor.extract(
            input, options: PDFExtractImagesOptions(minSize: 1), pageRangeText: nil, to: .alongsideInput
        )

        #expect(result.outputs.isEmpty)
        #expect(result.unsupported == 1)
    }

    // MARK: - De-duplication

    @Test func logoSharedByTwoPagesExtractedOnce() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.rawPDF(named: "shared-doc", objects: [
            RawObject.catalog,
            RawObject.pages(kids: [3, 4]),
            RawObject.page(id: 3, contents: 5, xobjects: "/Im0 9 0 R"),
            RawObject.page(id: 4, contents: 6, xobjects: "/Im0 9 0 R"),
            RawObject.content(id: 5, ops: "q 30 0 0 30 10 10 cm /Im0 Do Q"),
            RawObject.content(id: 6, ops: "q 90 0 0 90 0 0 cm /Im0 Do Q"),
            (
                9,
                "<< /Type /XObject /Subtype /Image /Width 3 /Height 3"
                    + " /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length __LENGTH__ >>",
                Data(Array(repeating: UInt8(128), count: 27))
            ),
        ])

        let result = try PDFImageExtractor.extract(
            input, options: PDFExtractImagesOptions(minSize: 1), pageRangeText: nil, to: .alongsideInput
        )

        #expect(result.outputs.map(\.lastPathComponent) == ["shared-doc-p1-1.png"])
        #expect(result.duplicates == 1)
    }

    // MARK: - Minimum size

    @Test func tinySpacersFilteredByMinimumSize() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.rawPDF(named: "tiny-doc", objects: [
            RawObject.catalog,
            RawObject.pages(kids: [3]),
            RawObject.page(id: 3, contents: 4, xobjects: "/Im0 5 0 R"),
            RawObject.content(id: 4, ops: "q 2 0 0 2 99 99 cm /Im0 Do Q"),
            (
                5,
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 2"
                    + " /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length __LENGTH__ >>",
                Data(repeating: 0x80, count: 12)
            ),
        ])

        let strict = try PDFImageExtractor.extract(input, options: PDFExtractImagesOptions(), pageRangeText: nil, to: .alongsideInput)
        #expect(strict.outputs.isEmpty)
        #expect(strict.belowMinSize == 1)

        let permissive = try PDFImageExtractor.extract(
            input, options: PDFExtractImagesOptions(minSize: 1), pageRangeText: nil, to: .alongsideInput
        )
        #expect(permissive.outputs.count == 1)
        #expect(permissive.belowMinSize == 0)
    }

    // MARK: - Page ranges and numbering

    @Test func pageRangeRestrictsSourcesAndNamesCarryPages() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.rawPDF(named: "range-doc", objects: [
            RawObject.catalog,
            RawObject.pages(kids: [3, 4]),
            RawObject.page(id: 3, contents: 5, xobjects: "/Im0 9 0 R"),
            RawObject.page(id: 4, contents: 6, xobjects: "/Im1 9 0 R /Im2 10 0 R"),
            RawObject.content(id: 5, ops: "q 30 0 0 30 0 0 cm /Im0 Do Q"),
            RawObject.content(id: 6, ops: "q 30 0 0 30 0 0 cm /Im1 Do Q\nq 30 0 0 30 60 60 cm /Im2 Do Q"),
            (
                9,
                "<< /Type /XObject /Subtype /Image /Width 3 /Height 3"
                    + " /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length __LENGTH__ >>",
                Data(repeating: 1, count: 27)
            ),
            (
                10,
                "<< /Type /XObject /Subtype /Image /Width 5 /Height 5"
                    + " /ColorSpace /DeviceGray /BitsPerComponent 8 /Length __LENGTH__ >>",
                Data(repeating: 2, count: 25)
            ),
        ])

        let result = try PDFImageExtractor.extract(input, options: PDFExtractImagesOptions(minSize: 1), pageRangeText: "2-2", to: .alongsideInput)

        #expect(result.outputs.map(\.lastPathComponent) == ["range-doc-p2-1.png", "range-doc-p2-2.png"])
        #expect(result.duplicates == 0)
    }

    // MARK: - Soft masks

    @Test func softMaskBecomesRealAlpha() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.rawPDF(named: "smask-doc", objects: [
            RawObject.catalog,
            RawObject.pages(kids: [3]),
            RawObject.page(id: 3, contents: 4, xobjects: "/Im0 5 0 R"),
            RawObject.content(id: 4, ops: "q 40 0 0 20 0 0 cm /Im0 Do Q"),
            (
                5,
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 1"
                    + " /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask 6 0 R /Length __LENGTH__ >>",
                Data([0xFF, 0x00, 0x00, 0xFF, 0x00, 0x00])
            ),
            (
                6,
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 1"
                    + " /ColorSpace /DeviceGray /BitsPerComponent 8 /Length __LENGTH__ >>",
                Data([0x00, 0xFF])
            ),
        ])

        let result = try PDFImageExtractor.extract(
            input, options: PDFExtractImagesOptions(minSize: 1), pageRangeText: nil, to: .alongsideInput
        )

        let output = try #require(result.outputs.first)
        #expect(result.outputs.count == 1)
        let hidden = try pixel(at: 0, 0, in: output)
        #expect(hidden.a == 0)
        let visible = try pixel(at: 1, 0, in: output)
        #expect(visible.a == 255)
        #expect(visible.r == 255 && visible.g == 0 && visible.b == 0)
    }

    // MARK: - Hand-built PDF plumbing

    private enum RawObject {
        static var catalog: (id: Int, head: String, stream: Data?) {
            (1, "<< /Type /Catalog /Pages 2 0 R >>", nil)
        }

        static func pages(kids: [Int]) -> (id: Int, head: String, stream: Data?) {
            let list = kids.map { "\($0) 0 R" }.joined(separator: " ")
            return (2, "<< /Type /Pages /Kids [\(list)] /Count \(kids.count) >>", nil)
        }

        static func page(id: Int, contents: Int, xobjects: String) -> (id: Int, head: String, stream: Data?) {
            (
                id,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200]"
                    + " /Resources << /XObject << \(xobjects) >> >> /Contents \(contents) 0 R >>",
                nil
            )
        }

        static func content(id: Int, ops: String) -> (id: Int, head: String, stream: Data?) {
            (id, "<< /Length __LENGTH__ >>", Data("\(ops)\n".utf8))
        }
    }

    // MARK: - zlib wrapper helpers

    /// Apple's codec speaks bare DEFLATE; PDF streams wear the RFC 1950
    /// wrapper, so the fixture wraps exactly like a producer would.
    private func zlibWrapped(_ raw: Data) -> Data {
        var wrapped = Data([0x78, 0x01])
        wrapped.append(deflated(raw))
        wrapped.append(contentsOf: withUnsafeBytes(of: adler32(raw).bigEndian) { Data($0) })
        return wrapped
    }

    private func deflated(_ data: Data) -> Data {
        var destination = [UInt8](repeating: 0, count: max(data.count * 2, 64))
        let written = compression_encode_buffer(
            &destination, destination.count,
            [UInt8](data), data.count,
            nil, COMPRESSION_ZLIB
        )
        precondition(written > 0, "deflate failed")
        return Data(destination.prefix(written))
    }

    private func adler32(_ data: Data) -> UInt32 {
        var a: UInt32 = 1
        var b: UInt32 = 0
        for byte in data {
            a = (a + UInt32(byte)) % 65521
            b = (b + a) % 65521
        }
        return (b << 16) | a
    }
}
