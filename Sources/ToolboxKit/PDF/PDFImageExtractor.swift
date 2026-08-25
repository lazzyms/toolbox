import Foundation
import CoreGraphics
import ImageIO
import CryptoKit
import UniformTypeIdentifiers

public struct PDFExtractImagesOptions: Sendable {
    /// Smallest edge an extracted image may have. Spacers, rules and other
    /// decoration is usually tiny; photographs are not.
    public var minSize: Int

    public init(minSize: Int = 32) {
        self.minSize = minSize
    }
}

/// What one extraction run produced. Skipped images are counted, never
/// thrown — one logo using JPXDecode must not fail the whole file.
public struct PDFImageExtraction: Sendable {
    /// Files written, in page order.
    public let outputs: [URL]
    /// Images dropped because the same bytes were already written.
    public let duplicates: Int
    /// Images dropped by the minimum-size filter.
    public let belowMinSize: Int
    /// Images whose encoding can't be honoured, left unwritten on purpose.
    public let unsupported: Int

    public init(outputs: [URL], duplicates: Int, belowMinSize: Int, unsupported: Int) {
        self.outputs = outputs
        self.duplicates = duplicates
        self.belowMinSize = belowMinSize
        self.unsupported = unsupported
    }
}

/// Pulls embedded image XObjects out of a PDF at their original resolution.
///
/// This walks each page's `/Resources /XObject` dictionaries with CoreGraphics
/// rather than rasterising: a photograph comes out as the photograph, not as
/// pixels of the page it was placed on (that is `PDFImageExporter`'s job).
///
/// Encoding support rides on what `CGPDFStreamCopyData` reports back:
/// - Streams it fully decodes (Flate, LZW, RunLength, ASCII85, PNG predictors)
///   arrive as raw samples and are rebuilt into PNG.
/// - DCT streams arrive as stored bytes and leave **byte-identical** as .jpg —
///   no re-encode, no quality loss.
/// - Anything CoreGraphics can't finish (JPX, CCITTFax, JBIG2) comes back in a
///   state we refuse to write, and is counted as unsupported instead.
public enum PDFImageExtractor {
    static let maxDimension = 16_000
    static let maxPixels = 80_000_000
    /// Form XObjects nest resources; a depth cap stops pathological nesting.
    private static let maxFormDepth = 8

    public static func extract(
        _ input: URL,
        options: PDFExtractImagesOptions,
        pageRangeText: String?,
        to location: OutputLocation
    ) throws -> PDFImageExtraction {
        if PDFUnlocker.isEncrypted(input) {
            throw ToolboxError.passwordProtected(input)
        }
        guard let document = CGPDFDocument(input as CFURL) else {
            throw ToolboxError.notAPDF(input)
        }
        let pageCount = document.numberOfPages
        guard pageCount > 0 else {
            throw ToolboxError.notAPDF(input)
        }

        let selected: [Int]
        if let rangeText = pageRangeText?.trimmingCharacters(in: .whitespaces), !rangeText.isEmpty {
            selected = try PageRange.parse(rangeText, pageCount: pageCount)
        } else {
            selected = Array(0..<pageCount)
        }

        var session = Session(
            options: options,
            stem: input.deletingPathExtension().lastPathComponent,
            directory: location.directory(forInput: input),
            location: location
        )

        // A worklist rather than recursion: form XObjects add their own
        // /Resources to the end of the queue, and the depth cap stops
        // pathological self-reference from looping forever.
        var queue: [(resources: CGPDFDictionaryRef, pageIndex: Int, depth: Int)] = []
        for pageIndex in selected {
            // The Swift overlay's CGPDFDocument.page(at:) is 1-based, unlike
            // every other page index in this codebase.
            guard let page = document.page(at: pageIndex + 1),
                  let pageDictionary = page.dictionary,
                  let resources = dictionary(pageDictionary, "Resources") else { continue }
            queue.append((resources, pageIndex, 0))
        }
        while !queue.isEmpty {
            let entry = queue.removeFirst()
            let foundStreams = xobjectStreams(in: entry.resources)
            for stream in foundStreams {
                let streamDictionary = CGPDFStreamGetDictionary(stream)!
                switch name(streamDictionary, "Subtype") {
                case "Image":
                    try handleImage(
                        stream, streamDictionary,
                        resources: entry.resources, pageIndex: entry.pageIndex, into: &session
                    )
                case "Form":
                    guard entry.depth < maxFormDepth,
                          let formResources = dictionary(streamDictionary, "Resources") else { continue }
                    queue.append((formResources, entry.pageIndex, entry.depth + 1))
                default:
                    break
                }
            }
        }

        return PDFImageExtraction(
            outputs: session.outputs,
            duplicates: session.duplicates,
            belowMinSize: session.belowMinSize,
            unsupported: session.unsupported
        )
    }

    // MARK: - Walking

    private struct Session {
        let options: PDFExtractImagesOptions
        let stem: String
        let directory: URL
        let location: OutputLocation

        var seen: Set<String> = []
        var outputs: [URL] = []
        var perPageCounts: [Int: Int] = [:]
        var duplicates = 0
        var belowMinSize = 0
        var unsupported = 0

        mutating func nextCandidate(pageIndex: Int, ext: String) -> URL {
            let number = (perPageCounts[pageIndex] ?? 0) + 1
            perPageCounts[pageIndex] = number
            // The name carries the source page so reruns collide into -1, -2…
            // instead of silently mixing runs together.
            return directory.appendingPathComponent("\(stem)-p\(pageIndex + 1)-\(number)")
        }
    }

    /// Collects every stream-valued entry of a resource dictionary's /XObject
    /// sub-dictionary. The applier is a C function pointer and cannot capture,
    /// so results travel through an explicitly managed box.
    private static func xobjectStreams(in resources: CGPDFDictionaryRef) -> [CGPDFStreamRef] {
        guard let xobjects = dictionary(resources, "XObject") else { return [] }
        let box = StreamBox()
        let info = Unmanaged.passUnretained(box).toOpaque()
        let applier: CGPDFDictionaryApplierFunction = { _, object, info in
            guard let info else { return }
            let box = Unmanaged<StreamBox>.fromOpaque(info).takeUnretainedValue()
            var found: CGPDFStreamRef?
            if CGPDFObjectGetValue(object, .stream, &found), let stream = found {
                box.streams.append(stream)
            }
        }
        CGPDFDictionaryApplyFunction(xobjects, applier, info)
        return box.streams
    }

    private final class StreamBox {
        var streams: [CGPDFStreamRef] = []
    }

    // MARK: - One image

    private enum Decoded {
        /// The embedded bytes already are a JPEG file; leave them untouched.
        case jpeg(Data)
        case bitmap(CGImage)
    }

    private static func handleImage(
        _ stream: CGPDFStreamRef,
        _ streamDictionary: CGPDFDictionaryRef,
        resources: CGPDFDictionaryRef?,
        pageIndex: Int,
        into session: inout Session
    ) throws {
        guard let width = int(streamDictionary, "Width"),
              let height = int(streamDictionary, "Height"),
              width > 0, height > 0 else {
            session.unsupported += 1
            return
        }
        // Stencil masks paint with the current fill colour — they aren't
        // pictures, and extracting one produces meaningless output.
        if bool(streamDictionary, "ImageMask") == true {
            session.unsupported += 1
            return
        }
        guard width <= maxDimension, height <= maxDimension, width * height <= maxPixels else {
            session.unsupported += 1
            return
        }

        guard let contents = streamContents(stream), !contents.data.isEmpty else {
            session.unsupported += 1
            return
        }

        // A header logo is one object drawn on many pages; hash the image's
        // own bytes so it lands once no matter how often it is referenced.
        let fingerprint = Self.fingerprint(
            width: width,
            height: height,
            bits: int(streamDictionary, "BitsPerComponent") ?? 8,
            data: contents.data
        )
        guard session.seen.insert(fingerprint).inserted else {
            session.duplicates += 1
            return
        }
        guard min(width, height) >= session.options.minSize else {
            session.belowMinSize += 1
            return
        }

        switch decode(contents: contents, streamDictionary: streamDictionary, resources: resources) {
        case .jpeg(let bytes):
            let candidate = session.nextCandidate(pageIndex: pageIndex, ext: "jpg")
            try writeJPEGBytes(bytes, to: candidate, location: session.location) { session.outputs.append($0) }
        case .bitmap(let base):
            // A soft mask that can't be recombined would leave a black box
            // where transparency belongs — that counts as unsupported.
            guard let image = applyingSoftMask(to: base, streamDictionary: streamDictionary, resources: resources) else {
                session.unsupported += 1
                return
            }
            let candidate = session.nextCandidate(pageIndex: pageIndex, ext: "png")
            try writeBitmap(image, to: candidate, location: session.location) { session.outputs.append($0) }
        case nil:
            session.unsupported += 1
        }
    }

    /// Interprets one image stream: either a stored JPEG or decodable samples.
    ///
    /// `CGPDFStreamCopyData` applies every encoding it understands before we
    /// see the data; its out `format` tells us honestly which happened.
    private static func decode(
        contents: (data: Data, format: CGPDFDataFormat),
        streamDictionary: CGPDFDictionaryRef,
        resources: CGPDFDictionaryRef?,
        assumeGrayWhenUnspecified: Bool = false
    ) -> Decoded? {
        let filters = filterNames(streamDictionary)
        switch contents.format {
        case .raw:
            // Fully decoded samples, ready for pixel reconstruction.
            return bitmap(
                samples: [UInt8](contents.data),
                streamDictionary: streamDictionary,
                resources: resources,
                assumeGrayWhenUnspecified: assumeGrayWhenUnspecified
            ).map(Decoded.bitmap)
        case .jpegEncoded:
            // Stored bytes survive only as a passthrough JPEG; anything else
            // written here would be a corrupt file, which this must never do.
            let isDCT = filters.contains("DCTDecode") || filters.contains("DCT")
            guard isDCT, contents.data.starts(with: [0xFF, 0xD8]) else { return nil }
            return .jpeg(contents.data)
        default:
            // CoreGraphics started decoding but hit an encoding it can't
            // finish (JPX, CCITTFax, JBIG2…). The bytes can't be trusted.
            return nil
        }
    }

    private static func fingerprint(width: Int, height: Int, bits: Int, data: Data) -> String {
        var seed = Data()
        for value in [UInt32(width), UInt32(height), UInt32(bits)] {
            seed.append(contentsOf: withUnsafeBytes(of: value.bigEndian) { Data($0) })
        }
        seed.append(data)
        return SHA256.hash(data: seed).map { String(format: "%02x", $0) }.joined()
    }

    // MARK: - Soft masks

    /// Combines an `/SMask` into real alpha. Without this a transparent logo
    /// would come out with a black box behind it. An image without an SMask
    /// comes back unchanged.
    private static func applyingSoftMask(
        to image: CGImage,
        streamDictionary: CGPDFDictionaryRef,
        resources: CGPDFDictionaryRef?
    ) -> CGImage? {
        guard let maskStream = stream(streamDictionary, "SMask") else { return image }
        let maskDictionary = CGPDFStreamGetDictionary(maskStream)!
        guard let maskWidth = int(maskDictionary, "Width"), let maskHeight = int(maskDictionary, "Height"),
              maskWidth == image.width, maskHeight == image.height,
              let maskContents = streamContents(maskStream) else { return nil }
        guard case .bitmap(let decoded)? = decode(
            contents: maskContents,
            streamDictionary: maskDictionary,
            resources: resources,
            assumeGrayWhenUnspecified: true
        ), let mask = toGrayscale(decoded), let masked = image.masking(mask) else { return nil }
        return flattened(masked)
    }

    private static func toGrayscale(_ image: CGImage) -> CGImage? {
        if image.colorSpace?.model == .monochrome && image.alphaInfo == .none {
            return image
        }
        guard let context = CGContext(
            data: nil, width: image.width, height: image.height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceGray(),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.none.rawValue)
        ) else { return nil }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return context.makeImage()
    }

    private static func flattened(_ image: CGImage) -> CGImage? {
        guard let context = CGContext(
            data: nil, width: image.width, height: image.height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue)
        ) else { return nil }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return context.makeImage()
    }

    // MARK: - Samples → CGImage

    private enum ColourKind {
        case gray
        case rgb
        case cmyk

        var components: Int {
            switch self {
            case .gray: return 1
            case .rgb: return 3
            case .cmyk: return 4
            }
        }
    }

    private enum ResolvedColour {
        case kind(ColourKind)
        case indexed(base: ColourKind, lookup: [UInt8], hival: Int)
    }

    private static func bitmap(
        samples raw: [UInt8],
        streamDictionary: CGPDFDictionaryRef,
        resources: CGPDFDictionaryRef?,
        assumeGrayWhenUnspecified: Bool = false
    ) -> CGImage? {
        guard let bits = int(streamDictionary, "BitsPerComponent"),
              let width = int(streamDictionary, "Width"),
              let height = int(streamDictionary, "Height") else { return nil }
        var colour = colour(of: streamDictionary, resources: resources)
        // An SMask's colour space is implied DeviceGray and often simply absent;
        // a colour-less main image stays unsupported rather than guessed at.
        if colour == nil && assumeGrayWhenUnspecified {
            colour = .kind(.gray)
        }
        guard let colour else { return nil }

        let resolved: (kind: ColourKind, samples: [UInt8])
        switch colour {
        case .kind(let kind):
            resolved = (kind, raw)
        case .indexed(let base, let lookup, let hival):
            // Palette entries expand into their base colours; low-bit indexed
            // packing is rare enough to skip honestly.
            guard bits == 8 else { return nil }
            guard hival >= 0, (hival + 1) * base.components <= lookup.count else { return nil }
            var expanded = [UInt8]()
            expanded.reserveCapacity(raw.count * base.components)
            for sample in raw {
                let entry = min(Int(sample), hival) * base.components
                expanded.append(contentsOf: lookup[entry..<entry + base.components])
            }
            resolved = (base, expanded)
        }

        let components = resolved.kind.components
        switch components {
        case 1: guard [1, 2, 4, 8, 16].contains(bits) else { return nil }
        case 3: guard [8, 16].contains(bits) else { return nil }
        default: guard bits == 8 else { return nil }
        }

        if resolved.kind == .cmyk {
            // CMYK has no lossless home in PNG; compositing through sRGB keeps
            // the picture viewable instead of writing something broken.
            guard bits == 8,
                  let cmyk = plane(width: width, height: height, bits: bits, components: 4, samples: resolved.samples),
                  let converted = convertToSRGB(cmyk) else { return nil }
            return converted
        }
        return plane(width: width, height: height, bits: bits, components: components, samples: resolved.samples)
    }

    private static func plane(width: Int, height: Int, bits: Int, components: Int, samples: [UInt8]) -> CGImage? {
        let stride = (width * components * bits + 7) / 8
        let needed = stride * height
        guard stride > 0, height > 0, samples.count >= needed else { return nil }
        guard let provider = CGDataProvider(data: Data(samples.prefix(needed)) as CFData) else { return nil }
        let space: CGColorSpace
        switch components {
        case 1: space = CGColorSpaceCreateDeviceGray()
        case 3: space = CGColorSpaceCreateDeviceRGB()
        case 4: space = CGColorSpaceCreateDeviceCMYK()
        default: return nil
        }
        return CGImage(
            width: width, height: height,
            bitsPerComponent: bits, bitsPerPixel: bits * components,
            bytesPerRow: stride, space: space,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.none.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .perceptual
        )
    }

    private static func convertToSRGB(_ image: CGImage) -> CGImage? {
        guard let context = CGContext(
            data: nil, width: image.width, height: image.height,
            bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue)
        ) else { return nil }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return context.makeImage()
    }

    // MARK: - Filters / colour spaces

    private static func filterNames(_ dictionary: CGPDFDictionaryRef) -> [String] {
        if let single = name(dictionary, "Filter") { return [single] }
        if let list = array(dictionary, "Filter") {
            var names: [String] = []
            for index in 0..<CGPDFArrayGetCount(list) {
                if let element = arrayName(list, index) { names.append(element) }
            }
            return names
        }
        return []
    }

    private static func colour(of streamDictionary: CGPDFDictionaryRef, resources: CGPDFDictionaryRef?) -> ResolvedColour? {
        if let single = name(streamDictionary, "ColorSpace") {
            return resolveColourName(single, resources: resources)
        }
        if let list = array(streamDictionary, "ColorSpace") {
            return resolveColourArray(list, resources: resources)
        }
        return nil
    }

    private enum ColourValue {
        case name(String)
        case array(CGPDFArrayRef)
    }

    private static func resolveColourName(_ colourName: String, resources: CGPDFDictionaryRef?) -> ResolvedColour? {
        switch colourName {
        case "DeviceGray", "CalGray", "G":
            return .kind(.gray)
        case "DeviceRGB", "CalRGB", "RGB":
            return .kind(.rgb)
        case "DeviceCMYK", "CMYK":
            return .kind(.cmyk)
        default:
            // Named colours like /Cs6 live in the page's /ColorSpace resource dict.
            guard let resourceColours = resources.flatMap({ dictionary($0, "ColorSpace") }),
                  let target = colourValue(in: resourceColours, key: colourName) else { return nil }
            return resolve(target, resources: resources)
        }
    }

    private static func resolveColourArray(_ array: CGPDFArrayRef, resources: CGPDFDictionaryRef?) -> ResolvedColour? {
        guard CGPDFArrayGetCount(array) >= 1, let family = arrayName(array, 0) else { return nil }
        switch family {
        case "ICCBased":
            guard let iccStream = arrayStream(array, 1) else { return nil }
            let iccDictionary = CGPDFStreamGetDictionary(iccStream)!
            switch int(iccDictionary, "N") {
            case 1: return .kind(.gray)
            case 3: return .kind(.rgb)
            case 4: return .kind(.cmyk)
            default: return nil
            }
        case "Indexed", "I":
            guard CGPDFArrayGetCount(array) >= 4,
                  let hival = arrayInt(array, 2),
                  hival >= 0,
                  let lookup = arrayBytes(array, 3) else { return nil }
            let base: ColourKind?
            if let baseName = arrayName(array, 1) {
                base = flatten(resolveColourName(baseName, resources: resources))
            } else if let baseArray = arrayArray(array, 1) {
                base = flatten(resolveColourArray(baseArray, resources: resources))
            } else {
                base = nil
            }
            guard let base else { return nil }
            return .indexed(base: base, lookup: lookup, hival: hival)
        case "CalRGB":
            return .kind(.rgb)
        case "CalGray":
            return .kind(.gray)
        default:
            // Lab, Separation, DeviceN and Pattern stay unsupported on purpose.
            return nil
        }
    }

    private static func flatten(_ resolved: ResolvedColour?) -> ColourKind? {
        guard case .kind(let kind) = resolved else { return nil }
        return kind
    }

    private static func resolve(_ value: ColourValue, resources: CGPDFDictionaryRef?) -> ResolvedColour? {
        switch value {
        case .name(let colourName): return resolveColourName(colourName, resources: resources)
        case .array(let array): return resolveColourArray(array, resources: resources)
        }
    }

    private static func colourValue(in dict: CGPDFDictionaryRef, key: String) -> ColourValue? {
        if let single = name(dict, key) { return .name(single) }
        if let list = array(dict, key) { return .array(list) }
        return nil
    }

    // MARK: - Output

    private static func writeJPEGBytes(_ bytes: Data, to candidate: URL, location: OutputLocation, record: (URL) -> Void) throws {
        let destination = OutputNaming.destination(
            for: candidate, in: location, extension: "jpg"
        )
        do {
            try bytes.write(to: destination, options: .atomic)
            record(destination)
        } catch {
            throw ToolboxError.writeFailed(destination)
        }
    }

    private static func writeBitmap(_ image: CGImage, to candidate: URL, location: OutputLocation, record: (URL) -> Void) throws {
        let destination = OutputNaming.destination(
            for: candidate, in: location, extension: "png"
        )
        do {
            try writePNG(image, to: destination)
            record(destination)
        } catch {
            throw ToolboxError.writeFailed(destination)
        }
    }

    private static func writePNG(_ image: CGImage, to url: URL) throws {
        guard let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.png.identifier as CFString, 1, nil
        ) else {
            throw ToolboxError.writeFailed(url)
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw ToolboxError.writeFailed(url)
        }
    }

    // MARK: - CGPDF plumbing

    private static func streamContents(_ stream: CGPDFStreamRef) -> (data: Data, format: CGPDFDataFormat)? {
        var format = CGPDFDataFormat.raw
        guard let copied = CGPDFStreamCopyData(stream, &format) else { return nil }
        return ((copied as Data?) ?? Data(), format)
    }

    private static func int(_ dictionary: CGPDFDictionaryRef, _ key: String) -> Int? {
        // GetInteger is no longer importable from Swift; GetNumber accepts
        // both integer and real values, so accept only integral numbers.
        var value: CGPDFReal = 0
        guard CGPDFDictionaryGetNumber(dictionary, key, &value) else { return nil }
        let rounded = value.rounded()
        guard abs(value - rounded) < 0.000001,
              rounded >= 0, rounded <= Double(Int.max) else { return nil }
        return Int(rounded)
    }

    private static func name(_ dictionary: CGPDFDictionaryRef, _ key: String) -> String? {
        var value: UnsafePointer<CChar>?
        guard CGPDFDictionaryGetName(dictionary, key, &value), let value else { return nil }
        return String(cString: value)
    }

    private static func bool(_ dictionary: CGPDFDictionaryRef, _ key: String) -> Bool? {
        var value = false
        guard CGPDFDictionaryGetBoolean(dictionary, key, &value) else { return nil }
        return value
    }

    private static func array(_ dictionary: CGPDFDictionaryRef, _ key: String) -> CGPDFArrayRef? {
        var value: CGPDFArrayRef?
        guard CGPDFDictionaryGetArray(dictionary, key, &value) else { return nil }
        return value
    }

    private static func dictionary(_ dictionary: CGPDFDictionaryRef, _ key: String) -> CGPDFDictionaryRef? {
        var value: CGPDFDictionaryRef?
        guard CGPDFDictionaryGetDictionary(dictionary, key, &value) else { return nil }
        return value
    }

    private static func stream(_ dictionary: CGPDFDictionaryRef, _ key: String) -> CGPDFStreamRef? {
        var value: CGPDFStreamRef?
        guard CGPDFDictionaryGetStream(dictionary, key, &value) else { return nil }
        return value
    }

    private static func arrayName(_ array: CGPDFArrayRef, _ index: Int) -> String? {
        var value: UnsafePointer<CChar>?
        guard CGPDFArrayGetName(array, index, &value), let value else { return nil }
        return String(cString: value)
    }

    private static func arrayInt(_ array: CGPDFArrayRef, _ index: Int) -> Int? {
        var value: CGPDFInteger = 0
        guard CGPDFArrayGetInteger(array, index, &value) else { return nil }
        return Int(value)
    }

    private static func arrayStream(_ array: CGPDFArrayRef, _ index: Int) -> CGPDFStreamRef? {
        var value: CGPDFStreamRef?
        guard CGPDFArrayGetStream(array, index, &value) else { return nil }
        return value
    }

    private static func arrayArray(_ array: CGPDFArrayRef, _ index: Int) -> CGPDFArrayRef? {
        var value: CGPDFArrayRef?
        guard CGPDFArrayGetArray(array, index, &value) else { return nil }
        return value
    }

    private static func arrayBytes(_ array: CGPDFArrayRef, _ index: Int) -> [UInt8]? {
        var object: CGPDFObjectRef?
        guard CGPDFArrayGetObject(array, index, &object), let object else { return nil }
        var string: CGPDFStringRef?
        guard CGPDFObjectGetValue(object, .string, &string), let string else { return nil }
        guard let bytes = CGPDFStringGetBytePtr(string) else { return nil }
        let length = CGPDFStringGetLength(string)
        // Index-walked rather than sliced: palette lookups are tiny, and this
        // sidesteps pointer-subrange quirks across SDKs.
        return (0..<length).map { bytes[$0] }
    }
}
