import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Reads and removes image metadata.
///
/// `summary` flattens what ImageIO reports about a file into display rows; the
/// kit returns data, the app renders it. `strip` writes a metadata-free copy
/// whose pixels are left alone — for JPEG it copies the compressed scan data
/// byte-for-byte through `CGImageDestinationCopyImageSource`, and only falls
/// back to a re-encode when that route refuses or fails verification.
public enum ImageMetadata: Sendable {

    /// How much metadata a strip removes.
    public enum StripMode: String, CaseIterable, Sendable, Identifiable {
        /// Every tag except the orientation needed to display the image right.
        case everything
        /// The common case: drop coordinates, keep camera settings.
        case locationOnly
        /// The photographer's case inverted: drop everything but attribution.
        case keepCopyright

        public var id: String { rawValue }

        public var label: String {
            switch self {
            case .everything: return "Everything"
            case .locationOnly: return "Location only"
            case .keepCopyright: return "All except copyright"
            }
        }
    }

    // MARK: - Viewer

    /// What `url` carries, flattened into rows sorted by group then key.
    ///
    /// Top-level facts (pixel size, colour model, DPI) land in the "Image"
    /// group; each nested dictionary ImageIO reports becomes its own group
    /// (EXIF, GPS, IPTC, TIFF, PNG, …).
    public static func summary(
        of url: URL
    ) throws -> [(group: String, key: String, value: String)] {
        let source = try open(url)
        return flattened(properties(at: 0, of: source))
    }

    // MARK: - Stripper

    /// Writes a copy of `url` with metadata removed per `mode`.
    ///
    /// The original is never touched, and nothing is overwritten: naming goes
    /// through `OutputNaming.destination`, so `photo.jpg` becomes
    /// `photo-stripped.jpg`, then `photo-stripped-1.jpg`, …
    public static func strip(
        from url: URL,
        to location: OutputLocation,
        mode: StripMode = .everything
    ) throws -> URL {
        guard ImageFormat.isReadable(url) else {
            throw ToolboxError.unsupportedInput(url.pathExtension)
        }
        let source = try open(url)

        let sourceProperties = properties(at: 0, of: source)
        let orientation = Self.orientation(of: sourceProperties)
        let format = inferredFormat(from: url, source: source)
        let ext = format.canEncode ? format.fileExtension : ImageFormat.png.fileExtension
        let output = OutputNaming.destination(for: url, in: location, suffix: "-stripped", extension: ext)

        // Privacy tool, so like PDFUnlocker this verifies rather than trusts:
        // the output is reopened and the promised removals are asserted before
        // the file is reported as written.
        let expectations = Expectations(sourceProperties: sourceProperties, orientation: orientation)

        if losslessStrip(
            input: url,
            source: source,
            output: output,
            typeIdentifier: format.utType.identifier,
            passes: losslessPasses(mode: mode, orientation: orientation, sourceProperties: sourceProperties)
        ), verified(output, against: expectations, mode: mode) {
            return output
        }
        try? FileManager.default.removeItem(at: output)

        try reencodedStrip(
            input: url,
            source: source,
            sourceProperties: sourceProperties,
            output: output,
            format: format,
            mode: mode,
            orientation: orientation
        )
        guard verified(output, against: expectations, mode: mode) else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.stripFailed(url)
        }
        return output
    }

    // MARK: - Lossless route

    /// One `CGImageDestinationCopyImageSource` call per pass. Every pass copies
    /// the compressed image data untouched and rewrites only metadata, so two
    /// passes still cost no quality.
    private static func losslessPasses(
        mode: StripMode,
        orientation: UInt32,
        sourceProperties: [CFString: Any]
    ) -> [[CFString: Any]] {
        switch mode {
        case .everything:
            // Excluding GPS replaces the whole tag set on formats that honour
            // it, which takes the orientation tag down with everything else —
            // so it is restored in a second pass, still without a re-encode.
            var passes: [[CFString: Any]] = [[kCGImageMetadataShouldExcludeGPS: true]]
            if orientation != 1 {
                passes.append([kCGImageDestinationOrientation: orientation])
            }
            return passes

        case .locationOnly:
            // Merging empty metadata keeps the source's tags and only drops
            // GPS — the one combination that spares EXIF/TIFF/IPTC here.
            return [[
                kCGImageMetadataShouldExcludeGPS: true,
                kCGImageDestinationMergeMetadata: true,
                kCGImageDestinationMetadata: CGImageMetadataCreateMutable(),
            ]]

        case .keepCopyright:
            // Replacing all tags with one carrying only attribution is itself
            // the strip; orientation comes back in a second pass.
            var passes: [[CFString: Any]] = [[
                kCGImageDestinationMetadata: attributionMetadata(of: sourceProperties),
            ]]
            if orientation != 1 {
                passes.append([kCGImageDestinationOrientation: orientation])
            }
            return passes
        }
    }

    /// Chains the passes through temporary files in the output's directory and
    /// cleans up behind itself on any failure.
    private static func losslessStrip(
        input: URL,
        source: CGImageSource,
        output: URL,
        typeIdentifier: String,
        passes: [[CFString: Any]]
    ) -> Bool {
        var created: [URL] = []
        // Each pass after the first reads what the previous one wrote.
        var currentInput = input

        for (index, pass) in passes.enumerated() {
            let target: URL
            if index == passes.count - 1 {
                target = output
            } else {
                target = output.deletingLastPathComponent()
                    .appendingPathComponent("meta-\(UUID().uuidString)")
                    .appendingPathExtension(output.pathExtension)
            }

            guard let passSource = CGImageSourceCreateWithURL(currentInput as CFURL, nil),
                  let destination = CGImageDestinationCreateWithURL(
                      target as CFURL, typeIdentifier as CFString,
                      CGImageSourceGetCount(source), nil
                  ),
                  CGImageDestinationCopyImageSource(
                      destination, passSource, pass as CFDictionary, nil
                  )
            else {
                try? FileManager.default.removeItem(at: target)
                for url in created { try? FileManager.default.removeItem(at: url) }
                return false
            }
            created.append(target)
            currentInput = target
        }
        return true
    }

    /// A metadata object holding only copyright/attribution tags that actually
    /// exist in `sourceProperties`.
    private static func attributionMetadata(of properties: [CFString: Any]) -> CGMutableImageMetadata {
        let metadata = CGImageMetadataCreateMutable()

        func carry(_ dictionary: CFString, _ key: CFString) {
            guard let dict = properties[dictionary] as? [CFString: Any],
                  let value = dict[key]
            else { return }
            // Attribution values are property-list objects (strings here), and
            // the C entry point wants the Core Foundation object itself.
            CGImageMetadataSetValueMatchingImageProperty(metadata, dictionary, key, value as AnyObject)
        }
        carry(kCGImagePropertyIPTCDictionary, kCGImagePropertyIPTCCopyrightNotice)
        carry(kCGImagePropertyIPTCDictionary, kCGImagePropertyIPTCCredit)
        carry(kCGImagePropertyTIFFDictionary, kCGImagePropertyTIFFArtist)
        carry(kCGImagePropertyTIFFDictionary, kCGImagePropertyTIFFCopyright)
        // EXIF's Copyright tag has no imported constant; matching by name is
        // what the property-based API expects anyway.
        carry(kCGImagePropertyExifDictionary, "Copyright" as CFString)
        return metadata
    }

    // MARK: - Re-encode route

    /// The fallback when the lossless route refuses a format or fails
    /// verification: decode → write with only the properties the mode keeps.
    private static func reencodedStrip(
        input: URL,
        source: CGImageSource,
        sourceProperties: [CFString: Any],
        output: URL,
        format: ImageFormat,
        mode: StripMode,
        orientation: UInt32
    ) throws {
        guard format.canEncode else {
            throw ToolboxError.unsupportedOutput(format.displayName)
        }

        // No operations and `.up` orientation: frames decode exactly as stored
        // and the tag rides along separately instead of being baked in — the
        // opposite of the resize rule, because these pixels are not rewritten.
        let transform = ImageTransform(
            operations: [],
            sourceSize: .zero,
            orientation: .up,
            allowUpscale: false
        )

        if let sequence = try ImageFrameSequence.read(
            from: source, input: input, requestedFormat: nil, transform: transform
        ) {
            try sequence.write(to: output, quality: reencodeQuality)
            return
        }

        guard let image = CGImageSourceCreateImageAtIndex(
            source, 0, [kCGImageSourceShouldCache: false] as CFDictionary
        ) else {
            throw ToolboxError.decodeFailed(input)
        }
        guard let destination = CGImageDestinationCreateWithURL(
            output as CFURL, format.utType.identifier as CFString, 1, nil
        ) else {
            throw ToolboxError.writeFailed(output)
        }

        var properties = keptProperties(mode: mode, orientation: orientation, of: sourceProperties)
        if format.supportsQuality {
            properties[kCGImageDestinationLossyCompressionQuality] = reencodeQuality
        }
        CGImageDestinationAddImage(
            destination, image, properties.isEmpty ? nil : properties as CFDictionary
        )
        guard CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.encodeFailed(format.displayName)
        }
    }

    private static let reencodeQuality = 0.9

    /// What a re-encode may carry over, per mode.
    private static func keptProperties(
        mode: StripMode,
        orientation: UInt32,
        of properties: [CFString: Any]
    ) -> [CFString: Any] {
        var kept: [CFString: Any] = [kCGImagePropertyOrientation: orientation]

        // DPI belongs to the pixels, not the privacy story, and losing it would
        // rescale print layouts.
        for key in [kCGImagePropertyDPIWidth, kCGImagePropertyDPIHeight] {
            if let value = properties[key] { kept[key] = value }
        }

        // Local rather than stored constants: CFString isn't Sendable, so a
        // global table of keys would be a concurrency hazard for no gain.
        let iptcLocationKeys: [CFString] = [
            kCGImagePropertyIPTCSubLocation,
            kCGImagePropertyIPTCCity,
            kCGImagePropertyIPTCProvinceState,
            kCGImagePropertyIPTCCountryPrimaryLocationCode,
            kCGImagePropertyIPTCCountryPrimaryLocationName,
        ]
        let tiffAttributionKeys: [CFString] = [
            kCGImagePropertyTIFFArtist,
            kCGImagePropertyTIFFCopyright,
        ]
        let iptcAttributionKeys: [CFString] = [
            kCGImagePropertyIPTCCopyrightNotice,
            kCGImagePropertyIPTCCredit,
        ]

        switch mode {
        case .everything:
            break

        case .locationOnly:
            // Camera settings live here and contain no coordinates; GPS is a
            // separate dictionary that is simply not carried.
            for key in [kCGImagePropertyExifDictionary, kCGImagePropertyTIFFDictionary] {
                if let value = properties[key] { kept[key] = value }
            }
            if var iptc = properties[kCGImagePropertyIPTCDictionary] as? [CFString: Any] {
                for key in iptcLocationKeys where iptc[key] != nil {
                    iptc[key] = nil
                }
                if !iptc.isEmpty { kept[kCGImagePropertyIPTCDictionary] = iptc }
            }

        case .keepCopyright:
            if let tiff = properties[kCGImagePropertyTIFFDictionary] as? [CFString: Any] {
                let attribution = tiff.filter { tiffAttributionKeys.contains($0.key) }
                if !attribution.isEmpty { kept[kCGImagePropertyTIFFDictionary] = attribution }
            }
            if let iptc = properties[kCGImagePropertyIPTCDictionary] as? [CFString: Any] {
                let attribution = iptc.filter { iptcAttributionKeys.contains($0.key) }
                if !attribution.isEmpty { kept[kCGImagePropertyIPTCDictionary] = attribution }
            }
            if let exif = properties[kCGImagePropertyExifDictionary] as? [CFString: Any],
               let copyright = exif["Copyright" as CFString] {
                kept[kCGImagePropertyExifDictionary] = ["Copyright" as CFString: copyright]
            }
        }

        return kept
    }

    // MARK: - Verification

    /// What must be true of a stripped output for its mode, captured from the
    /// source before anything is written.
    private struct Expectations {
        let make: String?
        let copyright: String?
        let dateTimeOriginal: String?
        let orientation: UInt32

        init(sourceProperties: [CFString: Any], orientation: UInt32) {
            let tiff = sourceProperties[kCGImagePropertyTIFFDictionary] as? [CFString: Any]
            let exif = sourceProperties[kCGImagePropertyExifDictionary] as? [CFString: Any]

            self.make = tiff?[kCGImagePropertyTIFFMake] as? String
            self.dateTimeOriginal = exif?[kCGImagePropertyExifDateTimeOriginal] as? String
            if let notice = (sourceProperties[kCGImagePropertyIPTCDictionary]
                as? [CFString: Any])?[kCGImagePropertyIPTCCopyrightNotice] as? String {
                copyright = notice
            } else if let tiffCopyright = tiff?[kCGImagePropertyTIFFCopyright] as? String {
                copyright = tiffCopyright
            } else {
                copyright = exif?["Copyright" as CFString] as? String
            }
            self.orientation = orientation
        }
    }

    private static func verified(
        _ url: URL,
        against expectations: Expectations,
        mode: StripMode
    ) -> Bool {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil)
                as? [CFString: Any]
        else { return false }

        // GPS is the whole point; every mode must lose it.
        guard properties[kCGImagePropertyGPSDictionary] == nil else { return false }

        let exif = properties[kCGImagePropertyExifDictionary] as? [CFString: Any]
        let tiff = properties[kCGImagePropertyTIFFDictionary] as? [CFString: Any]

        // An absent orientation reads as upright, which is also how the source
        // default is interpreted.
        let outputOrientation = (properties[kCGImagePropertyOrientation] as? NSNumber)?
            .uint32Value ?? 1
        guard outputOrientation == expectations.orientation else { return false }

        switch mode {
        case .everything:
            guard exif?[kCGImagePropertyExifDateTimeOriginal] == nil else { return false }
            return tiff?[kCGImagePropertyTIFFMake] == nil
                && exif?["Make" as CFString] == nil

        case .locationOnly:
            // The promise cuts both ways: coordinates go, camera settings stay.
            if let make = expectations.make {
                return tiff?[kCGImagePropertyTIFFMake] as? String == make
            }
            return true

        case .keepCopyright:
            guard tiff?[kCGImagePropertyTIFFMake] == nil
                && exif?["Make" as CFString] == nil
            else { return false }
            if let copyright = expectations.copyright {
                let iptc = properties[kCGImagePropertyIPTCDictionary] as? [CFString: Any]
                return iptc?[kCGImagePropertyIPTCCopyrightNotice] as? String == copyright
                    || tiff?[kCGImagePropertyTIFFCopyright] as? String == copyright
                    || exif?["Copyright" as CFString] as? String == copyright
            }
            return true
        }
    }

    // MARK: - Reading helpers

    private static func open(_ url: URL) throws -> CGImageSource {
        // kCGImageSourceShouldCache false: each pixel buffer is touched once,
        // so caching only inflates memory during batch runs.
        guard let source = CGImageSourceCreateWithURL(
            url as CFURL,
            [kCGImageSourceShouldCache: false] as CFDictionary
        ), CGImageSourceGetCount(source) > 0
        else {
            throw ToolboxError.decodeFailed(url)
        }
        return source
    }

    private static func properties(at index: Int, of source: CGImageSource) -> [CFString: Any] {
        (CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any]) ?? [:]
    }

    private static func orientation(of properties: [CFString: Any]) -> UInt32 {
        (properties[kCGImagePropertyOrientation] as? NSNumber)?.uint32Value ?? 1
    }

    /// Mirrors `ImageProcessor`'s inference: the container's own type first,
    /// then the extension, then PNG as the safe answer.
    private static func inferredFormat(from url: URL, source: CGImageSource) -> ImageFormat {
        if let type = CGImageSourceGetType(source) as String?,
           let match = ImageFormat.allCases.first(where: { $0.utType.identifier == type }) {
            return match
        }
        let ext = url.pathExtension.lowercased()
        return ImageFormat.allCases.first { $0.fileExtension == ext } ?? .png
    }

    // MARK: - Flattening

    private static func flattened(
        _ properties: [CFString: Any]
    ) -> [(group: String, key: String, value: String)] {
        var rows: [(group: String, key: String, value: String)] = []
        for (key, value) in properties {
            let name = key as String
            if name.hasPrefix("{"), name.hasSuffix("}"),
               let nested = value as? [CFString: Any] {
                let group = Self.group(String(name.dropFirst().dropLast()))
                for (innerKey, innerValue) in nested {
                    rows.append((group, innerKey as String, describe(innerValue)))
                }
            } else {
                rows.append(("Image", name, describe(value)))
            }
        }
        return rows.sorted {
            $0.group == $1.group ? $0.key < $1.key : $0.group < $1.group
        }
    }

    private static func group(_ raw: String) -> String {
        switch raw {
        case "Exif": return "EXIF"
        case "MakerApple": return "Maker (Apple)"
        default: return raw
        }
    }

    private static func describe(_ value: Any) -> String {
        // A CFBoolean bridges in as an NSNumber that also claims Bool, so the
        // Core Foundation type ID is checked before numeric formatting shows
        // "1" where the file said true.
        if CFGetTypeID(value as CFTypeRef) == CFBooleanGetTypeID() {
            return (value as! CFBoolean) == kCFBooleanTrue ? "Yes" : "No"
        }
        switch value {
        case let number as NSNumber: return number.stringValue
        case let string as String: return string.trimmingCharacters(in: .whitespacesAndNewlines)
        case let array as [Any]: return array.map(describe).joined(separator: ", ")
        case let data as Data: return "<\(data.count) bytes>"
        default: return String(describing: value)
        }
    }
}
