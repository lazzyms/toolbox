import CoreGraphics
import Foundation
import ImageIO
import Testing
@testable import ToolboxKit

@Suite("ImageMetadata")
struct ImageMetadataTests {

    // MARK: - Viewer

    @Test("summary surfaces GPS and camera metadata in sorted order")
    func summarySurfacesGPSAndCamera() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.taggedImage(named: "photo")

        let rows = try ImageMetadata.summary(of: input)

        #expect(rows.contains { $0.group == "GPS" && $0.key == "Latitude" && $0.value == "37.3349" })
        #expect(rows.contains { $0.group == "GPS" && $0.key == "LongitudeRef" && $0.value == "W" })
        #expect(rows.contains { $0.group == "TIFF" && $0.key == "Make" && $0.value == "ProbeCam" })
        #expect(rows.contains { $0.group == "EXIF" && $0.key == "LensModel" && $0.value == "TestLens 50mm" })
        #expect(rows.contains { $0.group == "IPTC" && $0.key == "CopyrightNotice" })

        // The UI lists the rows as-is, so the order must already be right.
        let sorted = rows.sorted(by: Self.isBefore)
        #expect(rows.count == sorted.count)
        #expect(rows.map(\.group) == sorted.map(\.group))
        #expect(rows.map(\.key) == sorted.map(\.key))
        #expect(!rows.isEmpty)
    }

    @Test("a clean image produces no sensitive groups")
    func summaryOfCleanImage() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "clean", width: 32, height: 32)

        let rows = try ImageMetadata.summary(of: input)

        // The encoder synthesises benign EXIF housekeeping (colour space,
        // pixel dimensions) even for files written without metadata; what
        // must never appear is anything that says who or where.
        #expect(!rows.contains { $0.group == "GPS" })
        #expect(!rows.contains { $0.group == "EXIF" && $0.key == "DateTimeOriginal" })
        #expect(!rows.contains { $0.group == "TIFF" && $0.key == "Make" })
    }

    // MARK: - Strip: everything

    @Test("strip everything removes GPS, camera and timestamp data")
    func stripsEverything() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.taggedImage(named: "leak")
        let originalBytes = try Data(contentsOf: input)

        let output = try ImageMetadata.strip(
            from: input, to: .directory(fixtures.directory), mode: .everything
        )

        #expect(output != input)
        #expect(output.lastPathComponent.hasSuffix("-stripped.jpg"))
        // The original is untouched, byte for byte.
        #expect(try Data(contentsOf: input) == originalBytes)

        let rows = try ImageMetadata.summary(of: output)
        #expect(!rows.contains { $0.group == "GPS" })
        #expect(!rows.contains { $0.group == "TIFF" && $0.key == "Make" })
        #expect(!rows.contains { $0.group == "EXIF" && $0.key == "DateTimeOriginal" })
    }

    @Test("stripping a JPEG leaves the compressed scan data untouched")
    func jpegStripDoesNotReencode() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.taggedImage(named: "lossless", width: 200, height: 150)

        let output = try ImageMetadata.strip(
            from: input, to: .directory(fixtures.directory), mode: .everything
        )

        #expect(Self.scanPayload(of: input) == Self.scanPayload(of: output))
        #expect(Fixtures.pixelSize(of: output) == Fixtures.pixelSize(of: input))
        #expect(Self.decodedPixels(of: input) == Self.decodedPixels(of: output))
    }

    @Test("orientation survives stripping")
    func orientationPreserved() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.orientedJPEG(named: "rotated")

        let output = try ImageMetadata.strip(
            from: input, to: .directory(fixtures.directory), mode: .everything
        )

        let source = CGImageSourceCreateWithURL(output as CFURL, nil)!
        let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        #expect((props?[kCGImagePropertyOrientation] as? NSNumber)?.uint32Value == 6)
        // Pixels were not rewritten either, so the tag still means the same thing.
        #expect(Fixtures.pixelSize(of: output) == CGSize(width: 40, height: 20))
    }

    // MARK: - Strip: granular modes

    @Test("location-only keeps camera settings and drops coordinates")
    func stripsLocationOnly() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.taggedImage(named: "where")

        let output = try ImageMetadata.strip(
            from: input, to: .directory(fixtures.directory), mode: .locationOnly
        )

        let rows = try ImageMetadata.summary(of: output)
        #expect(!rows.contains { $0.group == "GPS" })
        #expect(rows.contains { $0.group == "TIFF" && $0.key == "Make" && $0.value == "ProbeCam" })
        #expect(rows.contains { $0.group == "EXIF" && $0.key == "DateTimeOriginal" })
    }

    @Test("keep-copyright preserves attribution and drops the rest")
    func stripsKeepingCopyright() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.taggedImage(named: "credit")

        let output = try ImageMetadata.strip(
            from: input, to: .directory(fixtures.directory), mode: .keepCopyright
        )

        let rows = try ImageMetadata.summary(of: output)
        #expect(!rows.contains { $0.group == "GPS" })
        #expect(!rows.contains { $0.group == "TIFF" && $0.key == "Make" })
        #expect(!rows.contains { $0.group == "EXIF" && $0.key == "DateTimeOriginal" })
        #expect(rows.contains {
            $0.key == "CopyrightNotice" && $0.value == "(c) Probe Photographer"
        })
    }

    // MARK: - Naming and errors

    @Test("never overwrites an existing stripped output")
    func noOverwrite() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.taggedImage(named: "dup")

        let first = try ImageMetadata.strip(
            from: input, to: .directory(fixtures.directory), mode: .everything
        )
        let second = try ImageMetadata.strip(
            from: input, to: .directory(fixtures.directory), mode: .everything
        )

        #expect(first != second)
        #expect(second.lastPathComponent == "dup-stripped-1.jpg")
        #expect(FileManager.default.fileExists(atPath: first.path))
    }

    @Test("undecodable input reports decodeFailed")
    func undecodableInput() throws {
        let fixtures = try Fixtures()
        let garbage = fixtures.directory.appendingPathComponent("junk.jpg")
        try Data("not an image".utf8).write(to: garbage)

        // Both entry points fail the same way: the file opens, but ImageIO
        // finds no image in it.
        #expect(throws: ToolboxError.decodeFailed(garbage)) {
            _ = try ImageMetadata.summary(of: garbage)
        }
        #expect(throws: ToolboxError.decodeFailed(garbage)) {
            _ = try ImageMetadata.strip(from: garbage, to: .alongsideInput)
        }
    }

    // MARK: - Formats

    @Test("HEIC comes out clean even when the lossless copy refuses")
    func heicStripsClean() throws {
        // Skip if this Mac can't produce a HEIC to test against.
        try #require(ImageFormat.heic.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.taggedImage(named: "phone", format: .heic)

        let output = try ImageMetadata.strip(
            from: input, to: .directory(fixtures.directory), mode: .everything
        )

        #expect(output.pathExtension == "heic")
        let rows = try ImageMetadata.summary(of: output)
        #expect(!rows.contains { $0.group == "GPS" })
        #expect(!rows.contains { $0.group == "TIFF" && $0.key == "Make" })
        #expect(Fixtures.pixelSize(of: output) == CGSize(width: 64, height: 48))
    }

    @Test("an animated GIF keeps every frame when stripped")
    func gifKeepsFrames() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.animatedGIF(named: "loop", delays: [0.1, 0.2, 0.3])

        let output = try ImageMetadata.strip(
            from: input, to: .directory(fixtures.directory), mode: .everything
        )

        let source = CGImageSourceCreateWithURL(output as CFURL, nil)!
        #expect(CGImageSourceGetCount(source) == 3)
        #expect(Fixtures.pixelSize(of: output) == CGSize(width: 64, height: 48))
    }

    // MARK: - Helpers

    private static func isBefore(
        _ lhs: (group: String, key: String, value: String),
        _ rhs: (group: String, key: String, value: String)
    ) -> Bool {
        if lhs.group != rhs.group { return lhs.group < rhs.group }
        return lhs.key < rhs.key
    }

    /// Everything after the JPEG start-of-scan marker: the entropy-coded image
    /// data itself. Identical bytes here mean the compressor never ran again.
    private static func scanPayload(of url: URL) -> Data {
        guard let data = try? Data(contentsOf: url) else { return Data() }
        var index = 2
        while index < data.count - 1 {
            if data[index] == 0xFF, data[index + 1] == 0xDA {
                return data.suffix(from: index + 2)
            }
            // Skip non-SOS marker segments by their declared length; fill and
            // restart bytes don't carry one.
            if data[index] == 0xFF, ![0xFF, 0x00, 0xD9].contains(data[index + 1]) {
                let length = Int(data[index + 2]) << 8 | Int(data[index + 3])
                index += 2 + length
            } else {
                index += 1
            }
        }
        return Data()
    }

    /// Decodes both files into identical RGBA buffers so a comparison says the
    /// pixels match rather than merely that the sizes do.
    private static func decodedPixels(of url: URL) -> Data {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
              let context = CGContext(
                  data: nil, width: image.width, height: image.height,
                  bitsPerComponent: 8, bytesPerRow: 0,
                  space: CGColorSpace(name: CGColorSpace.sRGB)!,
                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
              )
        else { return Data() }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return Data(bytes: context.data!, count: context.bytesPerRow * context.height)
    }
}
