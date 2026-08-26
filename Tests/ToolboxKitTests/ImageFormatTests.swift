import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import ToolboxKit

@Suite("ImageFormat")
struct ImageFormatTests {

    @Test("no format is offered in the UI that this Mac can't write")
    func offeredAreWritable() {
        for format in ImageFormat.encodable {
            #expect(format.canEncode)
        }
    }

    @Test("WebP is read but never offered — ImageIO has no WebP encoder")
    func webpNeverOffered() {
        #expect(ImageFormat.webp.canEncode == false)
        #expect(!ImageFormat.encodable.contains(.webp))
        #expect(ImageFormat.isReadable(URL(fileURLWithPath: "/tmp/photo.webp")))
    }

    @Test("AVIF encodes where the OS supports it")
    func avifEncodes() throws {
        try #require(ImageFormat.avif.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "photo", width: 320, height: 240, format: .png)

        let result = try ImageProcessor().run(
            input, options: .init(format: .avif, location: .directory(fixtures.directory))
        )

        #expect(result.output.pathExtension == "avif")
        #expect(Fixtures.format(of: result.output) == ImageFormat.avif.utType.identifier)
    }

    @Test("GIF encodes a still image as a single frame")
    func gifEncodesStill() throws {
        try #require(ImageFormat.gif.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "photo", width: 320, height: 240, format: .png)

        let result = try ImageProcessor().run(
            input, options: .init(format: .gif, location: .directory(fixtures.directory))
        )

        #expect(result.output.pathExtension == "gif")
        #expect(Fixtures.format(of: result.output) == ImageFormat.gif.utType.identifier)
    }

    @Test("converting an animated GIF to GIF keeps every frame")
    func animatedGifToGifKeepsFrames() throws {
        try #require(ImageFormat.gif.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.animatedGIF(named: "walk", delays: [0.1, 0.25, 0.4])

        let result = try ImageProcessor().run(
            input, options: .init(format: .gif, location: .directory(fixtures.directory))
        )

        #expect(result.output.pathExtension == "gif")
        guard let source = CGImageSourceCreateWithURL(result.output as CFURL, nil) else {
            Issue.record("output is unreadable")
            return
        }
        #expect(CGImageSourceGetCount(source) == 3)
    }

    @Test("ICO writes where the OS supports it")
    func icoWrites() throws {
        try #require(ImageFormat.ico.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "mark", width: 64, height: 64, format: .png)

        let result = try ImageProcessor().run(
            input, options: .init(format: .ico, location: .directory(fixtures.directory))
        )

        #expect(result.output.pathExtension == "ico")
    }

    @Test("ICNS writes where the OS supports it")
    func icnsWrites() throws {
        try #require(ImageFormat.icns.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "mark", width: 128, height: 128, format: .png)

        let result = try ImageProcessor().run(
            input, options: .init(format: .icns, location: .directory(fixtures.directory))
        )

        #expect(result.output.pathExtension == "icns")
    }

    @Test("a quality slider applies only to lossy formats")
    func qualitySlider() {
        #expect(ImageFormat.jpeg.supportsQuality)
        #expect(ImageFormat.heic.supportsQuality)
        #expect(ImageFormat.avif.supportsQuality)
        #expect(!ImageFormat.gif.supportsQuality)
        #expect(!ImageFormat.tiff.supportsQuality)
        #expect(!ImageFormat.ico.supportsQuality)
        #expect(!ImageFormat.icns.supportsQuality)
    }

    @Test("readable extensions cover the added formats and sequence variants")
    func readableExtensionsComplete() {
        for ext in ["heics", "avifs", "ico", "icns", "jp2", "j2k", "rw2", "srw", "pef"] {
            #expect(ImageFormat.readableExtensions.contains(ext), "missing readable extension \(ext)")
        }
    }

    @Test("the compress picker's lossy list only contains lossy formats")
    func lossyList() {
        for format in ImageFormat.encodable where !format.isLossless {
            #expect(format.supportsQuality)
        }
    }
}
