import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import ToolboxKit

/// Pins the face-blur tool's promises without betting on Vision: the pixel
/// maths collapses local contrast exactly where a rect lands (and not in its
/// mirror — the flip bug #34 calls out), radii scale with face size, outputs
/// get the "-blurred" suffix through `OutputNaming`, originals stay untouched,
/// animated inputs are refused, and an honest run that finds nothing writes
/// nothing. One lenient contract test runs real detection and accepts either
/// outcome, because synthetic drawings can't guarantee detections.
@Suite("ImageFaceBlurrer")
struct ImageFaceBlurrerTests {

    // MARK: - The pixel maths

    @Test("blurring a region collapses its local contrast and leaves the rest sharp")
    func blurCollapsesContrastOnlyInsideRect() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "texture", width: 120, height: 80)
        let original = try Fixtures.cgImage(of: input)
        let before = Self.pixels(of: original)

        // Display coordinates: the left half, top-left origin like every
        // caller speaks.
        let leftHalf = CGRect(x: 0, y: 0, width: 60, height: 80)
        let blurred = try ImageFaceBlurrer.blur(rects: [leftHalf], in: original, radius: 8)
        let after = Self.pixels(of: blurred)

        let insideBefore = Self.meanSecondDifference(
            before, width: original.width,
            region: CGRect(x: 6, y: 10, width: 46, height: 60)
        )
        let insideAfter = Self.meanSecondDifference(
            after, width: original.width,
            region: CGRect(x: 6, y: 10, width: 46, height: 60)
        )
        let outsideBefore = Self.meanSecondDifference(
            before, width: original.width,
            region: CGRect(x: 68, y: 10, width: 46, height: 60)
        )
        let outsideAfter = Self.meanSecondDifference(
            after, width: original.width,
            region: CGRect(x: 68, y: 10, width: 46, height: 60)
        )

        // The fixture's blocky fill really has local structure to destroy;
        // second differences ignore the smooth ramps a blur legitimately
        // keeps and measure only the detail it must kill.
        #expect(insideBefore > 1)
        #expect(insideAfter < insideBefore / 3)
        // Compositing goes through the untouched source, so sharpness out
        // there survives the round trip to within rounding noise.
        #expect(abs(outsideAfter - outsideBefore) < 1.5)
    }

    @Test("a rect blurs the quadrant you asked for, not its mirror")
    func blurLandsInRequestedQuadrant() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.quadrantImage(named: "place", width: 64, height: 48)
        let original = try Fixtures.cgImage(of: input)

        // Top-right as displayed (green); if the coordinate flip were wrong
        // the tool would mangle the top-left instead.
        let topRight = CGRect(x: 32, y: 0, width: 32, height: 24)
        let blurred = try ImageFaceBlurrer.blur(rects: [topRight], in: original, radius: 10)
        let after = Self.pixels(of: blurred)
        let width = original.width

        let topLeft = Self.pixel(after, width: width, x: 16, y: 12)
        let topRightAfter = Self.pixel(after, width: width, x: 48, y: 12)
        let bottomLeft = Self.pixel(after, width: width, x: 16, y: 36)
        let bottomRight = Self.pixel(after, width: width, x: 48, y: 36)

        // Untouched quadrants keep their flat colours to within render noise.
        #expect(abs(Int(topLeft.r) - 255) <= 3 && topLeft.g <= 3 && topLeft.b <= 3)
        #expect(bottomLeft.r <= 3 && bottomLeft.g <= 3 && abs(Int(bottomLeft.b) - 255) <= 3)
        #expect(abs(Int(bottomRight.r) - 255) <= 3 && abs(Int(bottomRight.g) - 255) <= 3 && bottomRight.b <= 3)
        // The blurred one pulled colour in from its neighbours.
        #expect(topRightAfter.r > 15)
    }

    @Test("an empty rect list returns the very same bitmap")
    func emptyRectsReturnInputUnchanged() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.quadrantImage(named: "noop", width: 32, height: 32)
        let original = try Fixtures.cgImage(of: input)

        let result = try ImageFaceBlurrer.blur(rects: [], in: original, radius: 20)

        #expect(Self.pixels(of: result) == Self.pixels(of: original))
    }

    @Test("blur radius scales with face size, floored by the slider")
    func effectiveRadiusScalesWithFaceSize() {
        // Small background figure: the slider floor wins.
        #expect(ImageFaceBlurrer.effectiveRadius(sliderRadius: 10, faceSize: CGSize(width: 40, height: 30)) == 10)
        // Large portrait: a quarter of the short side takes over.
        #expect(ImageFaceBlurrer.effectiveRadius(sliderRadius: 10, faceSize: CGSize(width: 400, height: 300)) == 75)
        // Monotonic in face size, never below the slider.
        var previous = 0.0
        for size in [20.0, 60.0, 120.0, 480.0] {
            let radius = ImageFaceBlurrer.effectiveRadius(sliderRadius: 8, faceSize: CGSize(width: size, height: size))
            #expect(radius >= max(8, previous))
            previous = radius
        }
    }

    @Test("detected boxes are padded by a fifth on each side")
    func expandedBoxPadsByFraction() {
        let box = CGRect(x: 100, y: 100, width: 40, height: 20)
        let padded = ImageFaceBlurrer.expanded(box)

        #expect(padded.minX == 92 && padded.maxX == 148)
        #expect(padded.minY == 96 && padded.maxY == 124)
        #expect(padded.contains(box))

        // An empty detection box must not grow into nonsense.
        #expect(ImageFaceBlurrer.expanded(.null).isEmpty)
    }

    // MARK: - File safety

    @Test("a run names its output -blurred, collides safely and never touches the original")
    func runNamesOutputsAndPreservesOriginal() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.quadrantImage(named: "photo", width: 64, height: 48)
        let before = try Data(contentsOf: input)
        let detector: @Sendable (CGImage) throws -> [CGRect] = { _ in
            [CGRect(x: 32, y: 0, width: 32, height: 24)]
        }

        let first = try ImageFaceBlurrer.run(input, location: .directory(fixtures.directory), detector: detector)
        let second = try ImageFaceBlurrer.run(input, location: .directory(fixtures.directory), detector: detector)

        #expect(first.output.lastPathComponent == "photo-blurred.png")
        #expect(second.output.lastPathComponent == "photo-blurred-1.png")
        #expect(first.faceCount == 1)
        #expect(try Data(contentsOf: input) == before)
    }

    @Test("a JPEG stays JPEG when this Mac can write JPEG", .enabled(if: ImageFormat.jpeg.canEncode))
    func runKeepsLossyFormat() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "phone", width: 64, height: 48, format: .jpeg)
        let detector: @Sendable (CGImage) throws -> [CGRect] = { _ in
            [CGRect(x: 0, y: 0, width: 30, height: 30)]
        }

        let result = try ImageFaceBlurrer.run(input, location: .directory(fixtures.directory), detector: detector)

        #expect(result.output.pathExtension == "jpg")
        #expect(Fixtures.format(of: result.output) == ImageFormat.jpeg.utType.identifier)
    }

    @Test("finding no faces fails the file and writes nothing")
    func runWithNoFacesWritesNothing() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.quadrantImage(named: "nobody", width: 48, height: 48)
        let detector: @Sendable (CGImage) throws -> [CGRect] = { _ in [] }

        #expect(throws: ToolboxError.noFacesDetected(input)) {
            try ImageFaceBlurrer.run(input, location: .directory(fixtures.directory), detector: detector)
        }

        let written = try FileManager.default.contentsOfDirectory(atPath: fixtures.directory.path)
            .filter { $0.contains("-blurred") }
        #expect(written.isEmpty)
    }

    @Test("an animated input is refused rather than silently flattened")
    func animatedInputIsRejected() throws {
        let fixtures = try Fixtures()
        let tiff = try fixtures.multipageTIFF(named: "anim", frames: 2)
        let detector: @Sendable (CGImage) throws -> [CGRect] = { _ in
            [CGRect(x: 0, y: 0, width: 10, height: 10)]
        }

        #expect(throws: ToolboxError.wouldDropFrames(tiff, frames: 2, format: "TIFF")) {
            try ImageFaceBlurrer.run(tiff, location: .directory(fixtures.directory), detector: detector)
        }
    }

    @Test("garbage input fails the file with a decode failure")
    func corruptInputFailsAsDecodeFailure() throws {
        let fixtures = try Fixtures()
        let bad = fixtures.directory.appendingPathComponent("broken").appendingPathExtension("png")
        try Data([0xDE, 0xAD, 0xBE, 0xEF]).write(to: bad)
        let detector: @Sendable (CGImage) throws -> [CGRect] = { _ in
            [CGRect(x: 0, y: 0, width: 10, height: 10)]
        }

        #expect(throws: ToolboxError.decodeFailed(bad)) {
            try ImageFaceBlurrer.run(bad, location: .directory(fixtures.directory), detector: detector)
        }
    }

    // MARK: - Real detection (lenient contract)

    @Test("real detection on a drawn face either finds boxes or admits it — never crashes")
    func visionContractOnDrawnFace() throws {
        let fixtures = try Fixtures()
        let face = Self.crudeFaceImage(side: 256)

        // Either outcome is legitimate here: model revisions come and go, and
        // a crude drawing guarantees nothing. What must hold is "runs and
        // returns sane boxes".
        let faces = (try? ImageFaceBlurrer.detectFaces(in: face)) ?? []
        for rect in faces {
            #expect(rect.width >= 1 && rect.height >= 1)
            #expect(rect.minX >= -1 && rect.minY >= -1)
            #expect(rect.maxX <= CGFloat(face.width) + 1 && rect.maxY <= CGFloat(face.height) + 1)
        }

        // Whatever detection found, the blur stage must handle its boxes.
        if !faces.isEmpty {
            let padded = faces.map { ImageFaceBlurrer.expanded($0) }
            let blurred = try ImageFaceBlurrer.blur(rects: padded, in: face, radius: 12)
            #expect(blurred.width == face.width)
            #expect(blurred.height == face.height)
        }

        // Full pipeline over a real file: succeed honestly or refuse to write,
        // nothing in between.
        let url = fixtures.directory.appendingPathComponent("drawn").appendingPathExtension("png")
        let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
        #expect(destination != nil)
        CGImageDestinationAddImage(try #require(destination), face, nil)
        #expect(CGImageDestinationFinalize(try #require(destination)))

        do {
            let result = try ImageFaceBlurrer.run(url, location: .directory(fixtures.directory))
            #expect(result.faceCount >= 0)
        } catch let error as ToolboxError {
            #expect(error == .noFacesDetected(url), "unexpected failure: \(error)")
        }
    }

    // MARK: - Reading pixels back

    /// Canonical sRGB RGBA bytes, top row first, so any two renders compare.
    private static func pixels(of image: CGImage) -> [UInt8] {
        let width = image.width
        let height = image.height
        var bytes = [UInt8](repeating: 0, count: width * height * 4)
        bytes.withUnsafeMutableBytes { raw in
            guard let context = CGContext(
                data: raw.baseAddress, width: width, height: height,
                bitsPerComponent: 8, bytesPerRow: width * 4,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else { return }
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        }
        return bytes
    }

    private struct Sample {
        let r: UInt8, g: UInt8, b: UInt8
    }

    /// Byte triple at display coordinates (top-left origin).
    private static func pixel(
        _ data: [UInt8], width: Int, x: Int, y: Int
    ) -> Sample {
        let offset = (y * width + x) * 4
        return Sample(r: data[offset], g: data[offset + 1], b: data[offset + 2])
    }

    /// Mean absolute second difference along x in `region` (display
    /// coordinates). Smooth ramps contribute nothing, block edges and noise
    /// spike — exactly the local detail a Gaussian blur must destroy.
    private static func meanSecondDifference(
        _ data: [UInt8], width: Int, region: CGRect
    ) -> Double {
        func luminance(atX x: Int, y: Int) -> Double {
            let offset = (y * width + x) * 4
            return Double(
                Int(data[offset]) + Int(data[offset + 1]) + Int(data[offset + 2])
            ) / 3
        }
        var total = 0.0
        var count = 0.0
        for y in Int(region.minY)..<Int(region.maxY) {
            for x in (Int(region.minX) + 1)..<(Int(region.maxX) - 1) {
                total += abs(luminance(atX: x + 1, y: y) - 2 * luminance(atX: x, y: y)
                    + luminance(atX: x - 1, y: y))
                count += 1
            }
        }
        guard count > 0 else { return 0 }
        return total / count
    }

    /// A crude cartoon face — skin ellipse, darker eyes and mouth — good
    /// enough to sometimes trip the detector and always good enough to blur.
    private static func crudeFaceImage(side: Int) -> CGImage {
        let context = CGContext(
            data: nil, width: side, height: side,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )!
        context.setFillColor(red: 0.93, green: 0.78, blue: 0.63, alpha: 1)
        context.fillEllipse(in: CGRect(x: 38, y: 38, width: side - 76, height: side - 76))
        context.setFillColor(red: 0.15, green: 0.1, blue: 0.08, alpha: 1)
        let eyeY = Double(side) * 0.42
        let eyeSize = Double(side) * 0.08
        context.fillEllipse(in: CGRect(
            x: Double(side) * 0.32, y: eyeY, width: eyeSize, height: eyeSize
        ))
        context.fillEllipse(in: CGRect(
            x: Double(side) * 0.60, y: eyeY, width: eyeSize, height: eyeSize
        ))
        context.fill(CGRect(
            x: Double(side) * 0.35, y: Double(side) * 0.62,
            width: Double(side) * 0.30, height: Double(side) * 0.05
        ))
        return context.makeImage()!
    }
}
