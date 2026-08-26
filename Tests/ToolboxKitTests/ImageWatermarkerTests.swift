import Testing
import Foundation
import CoreGraphics
import UniformTypeIdentifiers
@testable import ToolboxKit

@Suite("Image Watermarker")
struct ImageWatermarkerTests {

    @Test func textLandsAtTheChosenAnchor() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.solidImage(
            named: "anchor-base", width: 240, height: 120, colour: (1, 1, 1)
        )

        let output = try ImageWatermarker.apply(
            ImageWatermarkOptions(
                content: .text("XXXX"),
                fontSize: .fraction(0.25),
                color: CGColor(red: 0, green: 0, blue: 0, alpha: 1),
                opacity: 1,
                rotationDegrees: 0,
                anchor: .bottomLeft
            ),
            to: input,
            destination: .alongsideInput
        )

        #expect(output.lastPathComponent == "anchor-base-watermarked.png")
        let px = try Fixtures.pixels(of: output)
        #expect(px.width == 240)
        #expect(px.height == 120)

        // The ink's bounding box must sit in the lower-left quadrant the
        // anchor names — display coordinates grow downward from the top.
        let box = try #require(px.inkBoundingBox(maxChannelBelow: 128))
        #expect(box.midX < CGFloat(px.width) / 2)
        #expect(box.midY > CGFloat(px.height) / 2)

        // Probe windows in display coordinates: one over where the stamp's
        // glyphs land, one high on the right that must stay pure base colour.
        let overInk = px.meanColour(in: CGRect(x: 12, y: 76, width: 40, height: 36))
        let awayFromInk = px.meanColour(in: CGRect(x: 180, y: 15, width: 50, height: 35))
        #expect(overInk.red < 190)
        #expect(overInk.red < awayFromInk.red - 60)
        #expect(awayFromInk.red > 240)
        #expect(awayFromInk.green > 240)
    }

    @Test func percentageSizingTracksImageWidth() throws {
        let fixtures = try Fixtures()
        let small = try fixtures.solidImage(
            named: "percent-small", width: 400, height: 200, colour: (1, 1, 1)
        )
        let large = try fixtures.solidImage(
            named: "percent-large", width: 800, height: 400, colour: (1, 1, 1)
        )

        func options() -> ImageWatermarkOptions {
            ImageWatermarkOptions(
                content: .text("WATERMARK"),
                fontSize: .fraction(0.08),
                color: CGColor(red: 0, green: 0, blue: 0, alpha: 1),
                opacity: 1,
                rotationDegrees: 0,
                anchor: .bottomLeft
            )
        }

        let smallOutput = try ImageWatermarker.apply(options(), to: small, destination: .alongsideInput)
        let largeOutput = try ImageWatermarker.apply(options(), to: large, destination: .alongsideInput)

        let smallBox = try #require(try Fixtures.pixels(of: smallOutput).inkBoundingBox(maxChannelBelow: 128))
        let largeBox = try #require(try Fixtures.pixels(of: largeOutput).inkBoundingBox(maxChannelBelow: 128))

        // Same fraction of a doubled width must double the rendered text; the
        // tolerance absorbs antialiasing and font hinting differences.
        let ratio = largeBox.width / smallBox.width
        #expect(ratio > 1.7)
        #expect(ratio < 2.3)
        // Neither render may be clipped by its canvas — that would make the
        // comparison meaningless.
        #expect(smallBox.width < 400)
        #expect(largeBox.width < 800)
    }

    @Test func logoAlphaIsPreservedAndStampCentres() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.solidImage(
            named: "logo-base", width: 200, height: 200, colour: (1, 1, 1)
        )
        let logo = try fixtures.transparentImage(named: "logo-stamp", side: 64)

        let output = try ImageWatermarker.apply(
            ImageWatermarkOptions(
                content: .image(logo),
                opacity: 1,
                rotationDegrees: 0,
                anchor: .center,
                imageScale: 0.5
            ),
            to: input,
            destination: .alongsideInput
        )

        let px = try Fixtures.pixels(of: output)
        // The logo is half the canvas wide and centred; its opaque core is the
        // middle half of itself, so red should fill the centre...
        let centre = px.meanColour(in: CGRect(x: 85, y: 85, width: 30, height: 30))
        #expect(centre.red > 150)
        #expect(centre.green < 100)
        #expect(centre.blue < 100)

        // ...while the transparent ring *inside* the drawn logo must leave the
        // white base untouched — proof alpha survived rather than flattening.
        let ring = px.meanColour(in: CGRect(x: 55, y: 55, width: 15, height: 15))
        #expect(ring.red > 235)
        #expect(ring.green > 235)
        #expect(ring.blue > 235)
    }

    @Test func tiledAnchorCoversEveryQuadrant() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.solidImage(
            named: "tiled-base", width: 300, height: 150, colour: (1, 1, 1)
        )

        let output = try ImageWatermarker.apply(
            ImageWatermarkOptions(
                content: .text("AB"),
                fontSize: .points(24),
                color: CGColor(red: 0, green: 0, blue: 0, alpha: 1),
                opacity: 1,
                rotationDegrees: 0,
                anchor: .tiled(spacing: 20)
            ),
            to: input,
            destination: .alongsideInput
        )

        let px = try Fixtures.pixels(of: output)
        // Inset windows so boundary-straddling tiles can't decide the result.
        let quadrants = [
            CGRect(x: 15, y: 15, width: 120, height: 45),     // top left
            CGRect(x: 165, y: 15, width: 120, height: 45),    // top right
            CGRect(x: 15, y: 90, width: 120, height: 45),     // bottom left
            CGRect(x: 165, y: 90, width: 120, height: 45),    // bottom right
        ]
        for window in quadrants {
            #expect(px.inkPixelCount(in: window, maxChannelBelow: 128) > 20)
        }
    }

    @Test func pngStaysPNG() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.solidImage(
            named: "format-png", width: 80, height: 60, colour: (0.9, 0.9, 0.9)
        )

        let output = try ImageWatermarker.apply(
            ImageWatermarkOptions(content: .text("DRAFT")),
            to: input,
            destination: .alongsideInput
        )

        #expect(output.pathExtension == "png")
        #expect(Fixtures.format(of: output) == UTType.png.identifier)
    }

    @Test func jpegKeepsFormatAndDimensionsWithQuality() throws {
        guard ImageFormat.jpeg.canEncode else { return }
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "format-jpeg", width: 96, height: 72, format: .jpeg)

        let output = try ImageWatermarker.apply(
            ImageWatermarkOptions(content: .text("DRAFT"), anchor: .center),
            to: input,
            destination: .alongsideInput,
            quality: 0.5
        )

        #expect(output.lastPathComponent == "format-jpeg-watermarked.jpg")
        #expect(Fixtures.format(of: output) == UTType.jpeg.identifier)
        let size = try #require(Fixtures.pixelSize(of: output))
        #expect(size.width == 96)
        #expect(size.height == 72)
    }

    @Test func heicStaysHEICWhenEncodable() throws {
        try #require(ImageFormat.heic.canEncode)
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "format-heic", width: 64, height: 48, format: .heic)

        let output = try ImageWatermarker.apply(
            ImageWatermarkOptions(content: .text("DRAFT")),
            to: input,
            destination: .alongsideInput
        )

        #expect(output.pathExtension == "heic")
        #expect(Fixtures.format(of: output) == UTType.heic.identifier)
    }

    @Test func undecodableInputFailsClearly() throws {
        let fixtures = try Fixtures()
        let junk = fixtures.directory.appendingPathComponent("junk.png")
        try Data("definitely not image data".utf8).write(to: junk)

        #expect(throws: ToolboxError.decodeFailed(junk)) {
            try ImageWatermarker.apply(
                ImageWatermarkOptions(content: .text("DRAFT")),
                to: junk,
                destination: .alongsideInput
            )
        }
    }

    @Test func animatedInputIsRejectedRatherThanSilentlyFlattened() throws {
        let fixtures = try Fixtures()
        let gif = try fixtures.animatedGIF(named: "watermark-anim", delays: [0.1, 0.25, 0.4])

        #expect(throws: ToolboxError.wouldDropFrames(gif, frames: 3, format: "GIF")) {
            try ImageWatermarker.apply(
                ImageWatermarkOptions(content: .text("DRAFT")),
                to: gif,
                destination: .alongsideInput
            )
        }
    }

    @Test func orientedJPEGIsUprightBeforeStamping() throws {
        let fixtures = try Fixtures()
        // 40×20 pixels with orientation 6: displayed as 20×40, top half red.
        let input = try fixtures.orientedJPEG(named: "oriented-base")

        let original = try Fixtures.pixels(of: input)
        #expect(original.width == 20)
        #expect(original.height == 40)

        let output = try ImageWatermarker.apply(
            ImageWatermarkOptions(
                content: .text("WW"),
                // Half the 20px displayed width keeps the glyphs inside the
                // blue lower band instead of spilling across both halves.
                fontSize: .fraction(0.5),
                color: CGColor(red: 0, green: 0, blue: 0, alpha: 1),
                opacity: 1,
                rotationDegrees: 0,
                anchor: .bottomLeft
            ),
            to: input,
            destination: .alongsideInput
        )

        // Portrait output proves the orientation was applied to the pixels;
        // no orientation tag may ride along or display would rotate it again.
        let size = try #require(Fixtures.pixelSize(of: output))
        #expect(size.width == 20)
        #expect(size.height == 40)

        let px = try Fixtures.pixels(of: output)
        // Displayed bottom-left is blue; the black stamp must dull it there…
        let stampedCorner = px.meanColour(in: CGRect(x: 1, y: 28, width: 16, height: 10))
        #expect(stampedCorner.blue < original.meanColour(in: CGRect(x: 1, y: 28, width: 16, height: 10)).blue - 30)
        // …and leave the displayed top (red) alone.
        let topBand = px.meanColour(in: CGRect(x: 1, y: 3, width: 16, height: 9))
        #expect(topBand.red > 200)
    }

    @Test func rerunNeverOverwrites() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.solidImage(
            named: "collide", width: 60, height: 40, colour: (1, 1, 1)
        )
        let options = ImageWatermarkOptions(content: .text("DRAFT"))

        let first = try ImageWatermarker.apply(options, to: input, destination: .alongsideInput)
        let second = try ImageWatermarker.apply(options, to: input, destination: .alongsideInput)

        #expect(first.lastPathComponent == "collide-watermarked.png")
        #expect(second.lastPathComponent == "collide-watermarked-1.png")
    }

    @Test func inputBytesAreNeverModified() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.solidImage(
            named: "untouched", width: 60, height: 40, colour: (1, 1, 1)
        )
        let before = try Data(contentsOf: input)

        _ = try ImageWatermarker.apply(
            ImageWatermarkOptions(content: .text("DRAFT")), to: input, destination: .alongsideInput
        )

        #expect(try Data(contentsOf: input) == before)
    }

    @Test func rejectsEmptyText() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.solidImage(
            named: "empty-text", width: 60, height: 40, colour: (1, 1, 1)
        )

        #expect(throws: ToolboxError.emptyWatermark) {
            try ImageWatermarker.apply(
                ImageWatermarkOptions(content: .text("   ")),
                to: input,
                destination: .alongsideInput
            )
        }
    }
}
