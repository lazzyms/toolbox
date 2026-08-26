import CoreGraphics
import Foundation
import ImageIO
import Testing
@testable import ToolboxKit

/// The rotate-and-flip tool is a thin layer over the pipeline's quarter-turn
/// rotation and mirroring, so the spec's job is to express the user's intent as
/// the right ordered operations — and to know when running it would change
/// nothing. These pin the wiring between that spec and the pipeline, which is
/// where a rotate tool tends to lose a flip or waste a render on a no-op.
@Suite("Rotate and flip")
struct RotateSpecTests {
    let processor = ImageProcessor()

    // MARK: - The spec

    @Test("a whole number of turns is not work at all")
    func wholeTurnsAreNoOps() {
        #expect(!RotateSpec().isActive)
        #expect(!RotateSpec(degrees: 360).isActive)
        #expect(!RotateSpec(degrees: -360).isActive)
        #expect(RotateSpec().operations.isEmpty)

        #expect(RotateSpec(degrees: 90).isActive)
        #expect(RotateSpec(degrees: 90, flipHorizontal: true).isActive)
        #expect(RotateSpec(flipVertical: true).isActive)
    }

    @Test("rotation is reported before the flips, in the order applied")
    func operationsAreOrdered() {
        #expect(RotateSpec(degrees: 90).operations == [.rotate(degrees: 90)])
        #expect(RotateSpec(flipHorizontal: true).operations == [.flip(horizontal: true, vertical: false)])
        #expect(RotateSpec(degrees: 90, flipHorizontal: true, flipVertical: true).operations == [
            .rotate(degrees: 90),
            .flip(horizontal: true, vertical: true),
        ])
    }

    // MARK: - Through the pipeline

    @Test("rotating 90° clockwise swaps the sides and moves the quadrants")
    func quarterTurnClockwise() throws {
        let fixtures = try Fixtures()
        // 80×40: red | green on top, blue | yellow underneath.
        let input = try fixtures.quadrantImage(named: "dial", width: 80, height: 40)

        let result = try processor.run(input, options: .init(
            operations: RotateSpec(degrees: 90).operations,
            suffix: "-r90", location: .directory(fixtures.directory)
        ))
        // A quarter turn clockwise swaps the sides and carries the top-left red
        // quadrant to the top right.
        #expect(Fixtures.pixelSize(of: result.output) == CGSize(width: 40, height: 80))
        #expect(Self.colour(result.output, x: 30, y: 10) == .red)
        #expect(Self.colour(result.output, x: 10, y: 10) == .blue)
    }

    @Test("flipping mirrors without changing dimensions")
    func flipMirrors() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.quadrantImage(named: "mirror", width: 80, height: 40)

        let result = try processor.run(input, options: .init(
            operations: RotateSpec(flipHorizontal: true).operations,
            suffix: "-hflip", location: .directory(fixtures.directory)
        ))
        #expect(Fixtures.pixelSize(of: result.output) == CGSize(width: 80, height: 40))
        // Mirroring brings the green top-right quadrant over to the left.
        #expect(Self.colour(result.output, x: 10, y: 10) == .green)
        #expect(Self.colour(result.output, x: 70, y: 10) == .red)
    }

    @Test("rotate then flip composes in the order the spec reports")
    func rotateThenFlip() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.quadrantImage(named: "combo", width: 80, height: 40)

        let result = try processor.run(input, options: .init(
            operations: RotateSpec(degrees: 90, flipHorizontal: true).operations,
            suffix: "-combo", location: .directory(fixtures.directory)
        ))
        // Rotate 90°: red goes top-right, blue top-left. Flip H: the top row
        // mirrors, so blue lands top-right and red top-left.
        #expect(Fixtures.pixelSize(of: result.output) == CGSize(width: 40, height: 80))
        #expect(Self.colour(result.output, x: 30, y: 10) == .blue)
        #expect(Self.colour(result.output, x: 10, y: 10) == .red)
    }

    @Test("an angle that isn't a quarter turn is refused, writing nothing")
    func arbitraryAngleRefused() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.image(named: "tilt", width: 60, height: 40)
        let before = try FileManager.default.contentsOfDirectory(atPath: fixtures.directory.path)

        #expect(throws: ToolboxError.unsupportedRotation(45)) {
            try processor.run(input, options: .init(
                operations: RotateSpec(degrees: 45).operations,
                location: .directory(fixtures.directory)
            ))
        }
        #expect(try FileManager.default.contentsOfDirectory(atPath: fixtures.directory.path) == before)
    }

    // MARK: - Reading files back

    private enum Colour: Equatable { case red, green, blue, yellow, other }

    private static func decoded(_ url: URL) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(source, 0, nil)
    }

    private static func pixels(of image: CGImage) -> Data {
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
        return Data(bytes)
    }

    /// The colour at one pixel, in top-left-origin coordinates.
    private static func colour(_ url: URL, x: Int, y: Int) -> Colour {
        guard let image = decoded(url), x < image.width, y < image.height else { return .other }
        let data = pixels(of: image)
        let offset = (y * image.width + x) * 4
        switch (data[offset] > 170, data[offset + 1] > 170, data[offset + 2] > 170) {
        case (true, false, false): return .red
        case (false, true, false): return .green
        case (false, false, true): return .blue
        case (true, true, false): return .yellow
        default: return .other
        }
    }
}
