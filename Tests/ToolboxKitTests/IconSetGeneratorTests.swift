import CoreGraphics
import Foundation
import ImageIO
import Testing
@testable import ToolboxKit

@Suite("IconSetGenerator")
struct IconSetGeneratorTests {

    @Test("macOS preset writes the full ladder and compiles it with iconutil")
    func macLadderIsComplete() throws {
        try #require(ImageFormat.ico.canEncode && ImageFormat.icns.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "mark", width: 1024, height: 1024, format: .png)

        let result = try IconSetGenerator.run(
            input, options: .init(preset: .macOS, location: .directory(fixtures.directory))
        )

        #expect(result.outputs.count == 11)
        let pngs = result.outputs.filter { $0.pathExtension == "png" }
        #expect(pngs.count == 10)
        for (pixels, filename) in IconSetGenerator.macLadder {
            let url = result.directory
                .appendingPathComponent("AppIcon.iconset")
                .appendingPathComponent(filename)
            #expect(FileManager.default.fileExists(atPath: url.path), "missing \(filename)")
            let size = try #require(Fixtures.pixelSize(of: url))
            #expect(Int(size.width) == pixels && Int(size.height) == pixels, Comment(rawValue: filename))
        }

        let icns = result.directory.appendingPathComponent("AppIcon.icns")
        guard let source = CGImageSourceCreateWithURL(icns as CFURL, nil) else {
            Issue.record("AppIcon.icns is unreadable")
            return
        }
        // iconutil keeps all ten representations; ImageIO's own ICNS encoder
        // drops five, which is why the tool doesn't use it.
        #expect(CGImageSourceGetCount(source) == 10)
    }

    @Test("favicon preset writes a multi-size ICO plus the common PNGs")
    func faviconPreset() throws {
        try #require(ImageFormat.ico.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "mark", width: 512, height: 512, format: .png)

        let result = try IconSetGenerator.run(
            input, options: .init(preset: .favicon, location: .directory(fixtures.directory))
        )

        let ico = result.directory.appendingPathComponent("favicon.ico")
        guard let source = CGImageSourceCreateWithURL(ico as CFURL, nil) else {
            Issue.record("favicon.ico is unreadable")
            return
        }
        #expect(CGImageSourceGetCount(source) == 3)
        var sizes: Set<Int> = []
        for index in 0..<CGImageSourceGetCount(source) {
            let props = CGImageSourceCopyPropertiesAtIndex(source, index, nil)
                as? [CFString: Any]
            let side = (props?[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue
            sizes.insert(side ?? -1)
        }
        #expect(sizes == [16, 32, 48])

        let touch = result.directory.appendingPathComponent("apple-touch-icon.png")
        let touchSize = try #require(Fixtures.pixelSize(of: touch))
        #expect(Int(touchSize.width) == 180)
        let chrome = result.directory.appendingPathComponent("android-chrome-512x512.png")
        let chromeSize = try #require(Fixtures.pixelSize(of: chrome))
        #expect(Int(chromeSize.width) == 512)
    }

    @Test("iOS preset writes every asset-catalogue size at the right pixels")
    func iOSPreset() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "mark", width: 1024, height: 1024, format: .png)

        let result = try IconSetGenerator.run(
            input, options: .init(preset: .iOS, location: .directory(fixtures.directory))
        )

        #expect(result.outputs.count == 11)
        let cases: [(filename: String, pixels: Int)] = [
            ("AppIcon-20@2x.png", 40),
            ("AppIcon-29@2x.png", 58),
            ("AppIcon-60@3x.png", 180),
            ("AppIcon-83.5@2x.png", 167),
            ("AppIcon-1024.png", 1024),
        ]
        for entry in cases {
            let url = result.directory.appendingPathComponent(entry.filename)
            let size = try #require(Fixtures.pixelSize(of: url))
            #expect(Int(size.width) == entry.pixels && Int(size.height) == entry.pixels,
                    Comment(rawValue: entry.filename))
        }
    }

    @Test("android preset writes a launcher icon per density plus the store icon")
    func androidPreset() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "mark", width: 512, height: 512, format: .png)

        let result = try IconSetGenerator.run(
            input, options: .init(preset: .android, location: .directory(fixtures.directory))
        )

        #expect(result.outputs.count == 6)
        let cases: [(path: String, pixels: Int)] = [
            ("mipmap-mdpi/ic_launcher.png", 48),
            ("mipmap-hdpi/ic_launcher.png", 72),
            ("mipmap-xhdpi/ic_launcher.png", 96),
            ("mipmap-xxhdpi/ic_launcher.png", 144),
            ("mipmap-xxxhdpi/ic_launcher.png", 192),
            ("play-store-icon.png", 512),
        ]
        for entry in cases {
            let url = result.directory.appendingPathComponent(entry.path)
            let size = try #require(Fixtures.pixelSize(of: url))
            #expect(Int(size.width) == entry.pixels && Int(size.height) == entry.pixels,
                    Comment(rawValue: entry.path))
        }
    }

    @Test("custom sizes are deduped, sorted and named icon-<size>.png")
    func customPreset() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "mark", width: 128, height: 128, format: .png)

        let result = try IconSetGenerator.run(
            input, options: .init(
                preset: .custom,
                customSizes: [100, 16, 16, 32],
                location: .directory(fixtures.directory)
            )
        )

        #expect(result.outputs.count == 3)
        let names = result.outputs.map(\.lastPathComponent).sorted()
        #expect(names == ["icon-100.png", "icon-16.png", "icon-32.png"])
        for name in names {
            let side = Int(name.dropFirst("icon-".count).dropLast(".png".count)) ?? 0
            let url = result.directory.appendingPathComponent(name)
            let size = try #require(Fixtures.pixelSize(of: url))
            #expect(Int(size.width) == side, Comment(rawValue: name))
        }
    }

    @Test("a custom preset with no usable sizes fails with a reason")
    func emptyCustomSizesThrows() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "mark", width: 64, height: 64, format: .png)

        #expect(throws: ToolboxError.invalidIconSizes(
            "Enter at least one size in pixels, e.g. “16, 32, 48”."
        )) {
            _ = try IconSetGenerator.run(
                input, options: .init(
                    preset: .custom,
                    customSizes: [0, -4],
                    location: .directory(fixtures.directory)
                )
            )
        }
    }

    @Test("crop to fill keeps the centre of a non-square source")
    func cropKeepsCentre() throws {
        let fixtures = try Fixtures()
        // 3:1 — red, green, blue in equal bands. A centred square crop keeps
        // only the green middle.
        let input = try fixtures.threeToneImage(named: "banner", width: 60, height: 20)

        let result = try IconSetGenerator.run(
            input, options: .init(
                preset: .custom,
                customSizes: [20],
                fill: .crop,
                location: .directory(fixtures.directory)
            )
        )

        let url = result.directory.appendingPathComponent("icon-20.png")
        let size = try #require(Fixtures.pixelSize(of: url))
        #expect(Int(size.width) == 20 && Int(size.height) == 20)
        let centre = try #require(pixel(in: url, atX: 10, y: 10))
        #expect(centre.g > 0.9 && centre.r < 0.1 && centre.b < 0.1)
    }

    @Test("stretch to fill squeezes the whole source to square")
    func stretchFillsSquare() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.threeToneImage(named: "banner", width: 60, height: 20)

        let result = try IconSetGenerator.run(
            input, options: .init(
                preset: .custom,
                customSizes: [20],
                fill: .stretch,
                location: .directory(fixtures.directory)
            )
        )

        let url = result.directory.appendingPathComponent("icon-20.png")
        let size = try #require(Fixtures.pixelSize(of: url))
        #expect(Int(size.width) == 20 && Int(size.height) == 20)
        // The red band is the source's left third, which a stretch squeezes into
        // the output's left ~6px — opposite of the crop test, where that band is
        // gone entirely.
        let left = try #require(pixel(in: url, atX: 3, y: 10))
        #expect(left.r > 0.9 && left.g < 0.1 && left.b < 0.1)
        let right = try #require(pixel(in: url, atX: 17, y: 10))
        #expect(right.b > 0.9 && right.r < 0.1 && right.g < 0.1)
    }

    @Test("alpha survives at every generated size")
    func alphaSurvives() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.transparentImage(named: "logo", side: 64)

        let result = try IconSetGenerator.run(
            input, options: .init(
                preset: .custom,
                customSizes: [16, 48],
                location: .directory(fixtures.directory)
            )
        )

        for name in ["icon-16.png", "icon-48.png"] {
            let url = result.directory.appendingPathComponent(name)
            let side = Int(name.dropFirst("icon-".count).dropLast(".png".count)) ?? 0
            let corner = try #require(pixel(in: url, atX: 1, y: 1))
            #expect(corner.a < 0.05, Comment(rawValue: "\(name) corner lost its transparency"))
            let centre = try #require(pixel(in: url, atX: side / 2, y: side / 2))
            #expect(centre.a > 0.95 && centre.r > 0.9, Comment(rawValue: name))
        }
    }

    @Test("a second run for the same source gets a fresh folder, not an overwrite")
    func secondRunGetsFreshFolder() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "mark", width: 64, height: 64, format: .png)

        let first = try IconSetGenerator.run(
            input, options: .init(preset: .custom, customSizes: [32],
                                  location: .directory(fixtures.directory))
        )
        let second = try IconSetGenerator.run(
            input, options: .init(preset: .custom, customSizes: [32],
                                  location: .directory(fixtures.directory))
        )

        #expect(second.directory != first.directory)
        #expect(second.directory.lastPathComponent == "mark-icons-1")
        #expect(FileManager.default.fileExists(atPath: first.directory.path))
        #expect(FileManager.default.fileExists(atPath: second.directory.path))
    }

    @Test("pixelSize reads declared dimensions without decoding")
    func declaredDimensions() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.threeToneImage(named: "banner", width: 60, height: 20)

        let size = try #require(IconSetGenerator.pixelSize(of: input))
        #expect(Int(size.width) == 60 && Int(size.height) == 20)
    }

    /// Reads one pixel back from a PNG on disk, y measured from the top.
    private func pixel(
        in url: URL, atX x: Int, y: Int
    ) -> (r: Double, g: Double, b: Double, a: Double)? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
        else { return nil }
        let width = image.width, height = image.height
        guard (0..<width).contains(x), (0..<height).contains(y) else { return nil }

        guard let context = CGContext(
            data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: width * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return nil }
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        guard let data = context.data else { return nil }
        let bytes = data.assumingMemoryBound(to: UInt8.self)
        let flipped = height - 1 - y
        let offset = (flipped * width + x) * 4
        return (
            Double(bytes[offset]) / 255,
            Double(bytes[offset + 1]) / 255,
            Double(bytes[offset + 2]) / 255,
            Double(bytes[offset + 3]) / 255
        )
    }
}
