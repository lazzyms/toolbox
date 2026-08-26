import Foundation
import ImageIO
import UniformTypeIdentifiers

public enum ImageFormat: String, CaseIterable, Sendable, Identifiable {
    case png, jpeg, heic, tiff, gif, avif, ico, icns, webp

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .png: return "PNG"
        case .jpeg: return "JPEG"
        case .heic: return "HEIC"
        case .tiff: return "TIFF"
        case .gif: return "GIF"
        case .avif: return "AVIF"
        case .ico: return "ICO"
        case .icns: return "ICNS"
        case .webp: return "WebP"
        }
    }

    public var fileExtension: String {
        switch self {
        case .png: return "png"
        case .jpeg: return "jpg"
        case .heic: return "heic"
        case .tiff: return "tiff"
        case .gif: return "gif"
        case .avif: return "avif"
        case .ico: return "ico"
        case .icns: return "icns"
        case .webp: return "webp"
        }
    }

    public var utType: UTType {
        switch self {
        case .png: return .png
        case .jpeg: return .jpeg
        case .heic: return .heic
        case .tiff: return .tiff
        case .gif: return .gif
        case .avif: return UTType("public.avif")!
        // No built-in statics for the icon containers, so their UTTypes come
        // from the registered identifiers ImageIO actually reports.
        case .ico: return UTType("com.microsoft.ico")!
        case .icns: return UTType("com.apple.icns")!
        case .webp: return .webP
        }
    }

    /// Whether a quality slider applies. Lossy formats (JPEG, HEIC, AVIF, WebP)
    /// get one; the rest don't — GIF quantises to a palette but has no
    /// continuous quality axis, and the icon containers are pixel-exact.
    public var isLossless: Bool {
        switch self {
        case .png, .tiff, .gif, .ico, .icns: return true
        case .jpeg, .heic, .avif, .webp: return false
        }
    }

    public var supportsQuality: Bool { !isLossless }

    /// Whether this Mac can actually *write* the format. WebP is excluded
    /// outright — ImageIO has never shipped a WebP encoder, so `.webp` is false
    /// on every macOS version and the UI never offers it. Every other format
    /// resolves from the system's current encoder list, so a newer macOS lights
    /// new options up without a code change.
    public var canEncode: Bool {
        (CGImageDestinationCopyTypeIdentifiers() as? [String])?
            .contains(utType.identifier) ?? false
    }

    public static var encodable: [ImageFormat] {
        allCases.filter(\.canEncode)
    }

    /// Extensions we accept as input. Broader than the encodable set because
    /// ImageIO reads more formats than it writes.
    ///
    /// Deliberately a hardcoded set rather than a reflection of
    /// `CGImageSourceCopyTypeIdentifiers()`: the derived list includes
    /// non-image types (PDF, PostScript) that must not appear in an image drop
    /// zone, and a fixed set keeps the drop zone predictable and testable. It
    /// is extended by hand as formats are added.
    public static let readableExtensions: Set<String> = [
        "heic", "heif", "heics", "png", "jpg", "jpeg", "tiff", "tif",
        "gif", "bmp", "webp", "avif", "avifs", "ico", "icns", "jp2", "j2k",
        "dng", "cr2", "nef", "arw", "orf", "raf", "rw2", "srw", "pef",
    ]

    public static func isReadable(_ url: URL) -> Bool {
        readableExtensions.contains(url.pathExtension.lowercased())
    }
}
