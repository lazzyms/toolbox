import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Generates a complete set of app icons and favicons from one source image.
///
/// One input becomes a folder of correctly-named files — the macOS `.icns`
/// ladder, a multi-size `favicon.ico` plus the common PNGs, the iOS or Android
/// app icon sets, or an arbitrary list of sizes. Every size goes through the
/// same pipeline `ImageProcessor` uses, so EXIF orientation is baked in and the
/// resize is Lanczos rather than the bilinear mush a plain `CGContext` scale
/// produces at 16px.
public enum IconSetGenerator {

    public struct Options: Sendable {
        public var preset: IconSetPreset
        /// Sizes for `.custom`; ignored by every other preset.
        public var customSizes: [Int]
        /// How a non-square source is turned into the square each icon needs.
        public var fill: IconFill
        public var location: OutputLocation

        public init(
            preset: IconSetPreset = .macOS,
            customSizes: [Int] = [],
            fill: IconFill = .crop,
            location: OutputLocation = .alongsideInput
        ) {
            self.preset = preset
            self.customSizes = customSizes
            self.fill = fill
            self.location = location
        }
    }

    public struct Result: Sendable {
        /// The folder that now holds every generated file.
        public let directory: URL
        /// Every file written, in the order written, for the results list.
        public let outputs: [URL]
    }

    /// Just the declared pixel dimensions — read from the header, so the pane
    /// can warn about an unsuitable source without a full decode.
    public static func pixelSize(of url: URL) -> CGSize? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return declaredSize(of: source)
    }

    public static func run(_ input: URL, options: Options) throws -> Result {
        guard ImageFormat.isReadable(input) else {
            throw ToolboxError.unsupportedInput(input.pathExtension)
        }

        let sourceOptions: [CFString: Any] = [kCGImageSourceShouldCache: false]
        guard let source = CGImageSourceCreateWithURL(input as CFURL, sourceOptions as CFDictionary),
              CGImageSourceGetCount(source) > 0
        else {
            throw ToolboxError.decodeFailed(input)
        }

        // Prefer the declared dimensions; they're cheap and correct before any
        // decode. Only a file that reports none falls back to decoding.
        var sourceSize = declaredSize(of: source)
        if sourceSize == nil {
            if let decoded = CGImageSourceCreateImageAtIndex(
                source, 0, sourceOptions as CFDictionary
            ) {
                sourceSize = CGSize(width: decoded.width, height: decoded.height)
            }
        }

        let plan = try Self.plan(for: options.preset, customSizes: options.customSizes)
        let directory = Self.destinationDirectory(
            for: input, in: options.location, suffix: plan.folderSuffix
        )
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        // One fresh pipeline per size. Building it per call keeps every size on
        // the identical decode → crop → Lanczos path, and the plan decides
        // whether the crop is even requested.
        func iconImage(at pixels: Int) throws -> CGImage {
            let transform = ImageTransform(
                operations: Self.operations(for: options.fill, pixels: pixels),
                sourceSize: sourceSize ?? .zero,
                orientation: Self.orientation(from: source),
                allowUpscale: true
            )
            return try transform.decode(from: source, at: 0, input: input).image
        }

        var outputs: [URL] = []

        for file in plan.pngs {
            let url = directory.appendingPathComponent(file.path)
            try FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(), withIntermediateDirectories: true
            )
            try Self.writePNG(iconImage(at: file.pixels), to: url)
            outputs.append(url)
        }

        if let ico = plan.ico {
            let url = directory.appendingPathComponent(ico.path)
            try Self.writeICO(sizes: ico.sizes, to: url, makeImage: iconImage)
            outputs.append(url)
        }

        if let icnsPath = plan.icns {
            let icns = directory.appendingPathComponent(icnsPath)
            try Self.compileICNS(
                from: directory.appendingPathComponent("AppIcon.iconset"),
                to: icns
            )
            outputs.append(icns)
        }

        return Result(directory: directory, outputs: outputs)
    }

    // MARK: - Presets

    /// The macOS `.iconset` ladder — the ten files `iconutil` compiles and that
    /// Xcode accepts as an asset catalogue entry.
    static let macLadder: [(pixels: Int, filename: String)] =
        [16, 32, 128, 256, 512].flatMap { base in
            [(base, "icon_\(base)x\(base).png"), (base * 2, "icon_\(base)x\(base)@2x.png")]
        }

    private struct IconSetPlan {
        let folderSuffix: String
        /// Individual PNG files: pixel size and path relative to the output folder.
        let pngs: [(pixels: Int, path: String)]
        /// One multi-size ICO: sizes and path. nil for presets without one.
        let ico: (sizes: [Int], path: String)?
        /// The `.icns` compiled from the iconset folder among `pngs`. nil when
        /// the preset writes no ICNS.
        let icns: String?
    }

    private static func plan(
        for preset: IconSetPreset, customSizes: [Int]
    ) throws -> IconSetPlan {
        switch preset {
        case .macOS:
            return IconSetPlan(
                folderSuffix: "-icons",
                pngs: macLadder.map { ($0.pixels, "AppIcon.iconset/\($0.filename)") },
                ico: nil,
                icns: "AppIcon.icns"
            )

        case .favicon:
            return IconSetPlan(
                folderSuffix: "-favicon",
                pngs: [
                    (16, "favicon-16x16.png"),
                    (32, "favicon-32x32.png"),
                    (48, "favicon-48x48.png"),
                    (180, "apple-touch-icon.png"),
                    (192, "android-chrome-192x192.png"),
                    (512, "android-chrome-512x512.png"),
                ],
                ico: (sizes: [16, 32, 48], path: "favicon.ico"),
                icns: nil
            )

        case .iOS:
            return IconSetPlan(
                folderSuffix: "-iOS",
                pngs: [
                    (40, "AppIcon-20@2x.png"),
                    (60, "AppIcon-20@3x.png"),
                    (58, "AppIcon-29@2x.png"),
                    (87, "AppIcon-29@3x.png"),
                    (80, "AppIcon-40@2x.png"),
                    (120, "AppIcon-40@3x.png"),
                    (120, "AppIcon-60@2x.png"),
                    (180, "AppIcon-60@3x.png"),
                    (152, "AppIcon-76@2x.png"),
                    (167, "AppIcon-83.5@2x.png"),
                    (1024, "AppIcon-1024.png"),
                ],
                ico: nil,
                icns: nil
            )

        case .android:
            return IconSetPlan(
                folderSuffix: "-Android",
                pngs: [
                    (48, "mipmap-mdpi/ic_launcher.png"),
                    (72, "mipmap-hdpi/ic_launcher.png"),
                    (96, "mipmap-xhdpi/ic_launcher.png"),
                    (144, "mipmap-xxhdpi/ic_launcher.png"),
                    (192, "mipmap-xxxhdpi/ic_launcher.png"),
                    (512, "play-store-icon.png"),
                ],
                ico: nil,
                icns: nil
            )

        case .custom:
            let sizes = Array(Set(customSizes.filter { $0 > 0 })).sorted()
            guard !sizes.isEmpty else {
                throw ToolboxError.invalidIconSizes(
                    "Enter at least one size in pixels, e.g. “16, 32, 48”."
                )
            }
            return IconSetPlan(
                folderSuffix: "-icons",
                pngs: sizes.map { ($0, "icon-\($0).png") },
                ico: nil,
                icns: nil
            )
        }
    }

    private static func operations(for fill: IconFill, pixels: Int) -> [ImageOperation] {
        switch fill {
        case .crop:
            return [
                .crop(.aspect(width: 1, height: 1, anchor: .center)),
                .resize(.exact(width: pixels, height: pixels)),
            ]
        case .stretch:
            return [.resize(.exact(width: pixels, height: pixels))]
        }
    }

    // MARK: - Encoding

    private static func writePNG(_ image: CGImage, to url: URL) throws {
        guard let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.png.identifier as CFString, 1, nil
        ) else {
            throw ToolboxError.writeFailed(url)
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: url)
            throw ToolboxError.encodeFailed("PNG")
        }
    }

    /// ICO is a multi-image container: one file holding every size. Each size is
    /// `AddImage`'d into the same destination, the same machinery the animated
    /// GIF path uses.
    private static func writeICO(
        sizes: [Int],
        to url: URL,
        makeImage: (Int) throws -> CGImage
    ) throws {
        let images = try sizes.map { pixels in
            (image: try makeImage(pixels), pixels: pixels)
        }
        guard let destination = CGImageDestinationCreateWithURL(
            url as CFURL, ImageFormat.ico.utType.identifier as CFString, images.count, nil
        ) else {
            throw ToolboxError.writeFailed(url)
        }
        for entry in images {
            CGImageDestinationAddImage(destination, entry.image, nil)
        }
        guard CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: url)
            throw ToolboxError.encodeFailed("ICO")
        }
    }

    /// The repo's own icon (`Scripts/make-icon.swift`) builds `AppIcon.icns`
    /// this way, and the tool deliberately stays consistent with it: a PNG
    /// ladder in an `.iconset` folder compiled by `/usr/bin/iconutil`. ImageIO's
    /// own ICNS encoder silently drops half the ladder — ten sizes in come back
    /// as five — so the system tool is the only path that yields the full set a
    /// retina app icon needs.
    private static func compileICNS(from iconset: URL, to output: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
        process.arguments = ["-c", "icns", iconset.path, "-o", output.path]
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw ToolboxError.encodeFailed("ICNS")
        }
    }

    // MARK: - Naming

    /// A fresh directory name that never overwrites an existing folder — the
    /// same collision-counter `OutputNaming` applies to files, since the output
    /// here is a directory rather than a single file.
    private static func destinationDirectory(
        for input: URL,
        in location: OutputLocation,
        suffix: String
    ) -> URL {
        let dir = location.directory(forInput: input)
        let base = input.deletingPathExtension().lastPathComponent + suffix
        var candidate = dir.appendingPathComponent(base)
        var counter = 1
        while FileManager.default.fileExists(atPath: candidate.path) {
            candidate = dir.appendingPathComponent("\(base)-\(counter)")
            counter += 1
            if counter > 9999 { break }
        }
        return candidate
    }

    // MARK: - Reading

    private static func declaredSize(of source: CGImageSource) -> CGSize? {
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil)
                as? [CFString: Any],
              let width = (properties[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue,
              let height = (properties[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue,
              width > 0, height > 0
        else { return nil }
        return CGSize(width: width, height: height)
    }

    private static func orientation(from source: CGImageSource) -> CGImagePropertyOrientation {
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil)
                as? [CFString: Any],
              let raw = (properties[kCGImagePropertyOrientation] as? NSNumber)?.uint32Value,
              let orientation = CGImagePropertyOrientation(rawValue: raw)
        else { return .up }
        return orientation
    }
}

/// Which app icon set to generate.
public enum IconSetPreset: String, CaseIterable, Identifiable, Sendable {
    case macOS, favicon, iOS, android, custom

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .macOS: return "macOS"
        case .favicon: return "Favicon"
        case .iOS: return "iOS"
        case .android: return "Android"
        case .custom: return "Custom sizes"
        }
    }

    public var summary: String {
        switch self {
        case .macOS:
            return "AppIcon.icns plus the full .iconset ladder — everything macOS needs, in one folder."
        case .favicon:
            return "favicon.ico (16/32/48 in one file), apple-touch-icon.png and the common web sizes."
        case .iOS:
            return "Every App Store size from 40×40 to 1024×1024, named for the asset catalogue."
        case .android:
            return "ic_launcher in each density mipmap folder, plus a 512px play-store icon."
        case .custom:
            return "One PNG per size you type."
        }
    }

    /// The largest output in pixels, or nil when the sizes aren't fixed. Used to
    /// warn before upscaling a source that is too small to look good.
    public func largestPixels(for customSizes: [Int]) -> Int? {
        switch self {
        case .macOS: return 1024
        case .favicon: return 512
        case .iOS: return 1024
        case .android: return 512
        case .custom: return customSizes.max()
        }
    }
}

/// How a non-square source is turned into the square each icon needs.
public enum IconFill: String, CaseIterable, Identifiable, Sendable {
    /// Centre the square on the source and cut the overflow off both edges.
    /// Nothing is distorted, but the sides of the artwork are lost.
    case crop
    /// Scale the whole source to the square. Nothing is lost, but the ratio
    /// is distorted unless it already matched.
    case stretch

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .crop: return "Crop to fill"
        case .stretch: return "Stretch to fill"
        }
    }
}
