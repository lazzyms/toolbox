import Foundation
import ImageIO
import UniformTypeIdentifiers

public enum ImageFormat: String, CaseIterable, Sendable, Identifiable {
    case png, jpeg, heic, tiff, webp

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .png: return "PNG"
        case .jpeg: return "JPEG"
        case .heic: return "HEIC"
        case .tiff: return "TIFF"
        case .webp: return "WebP"
        }
    }

    public var fileExtension: String {
        switch self {
        case .png: return "png"
        case .jpeg: return "jpg"
        case .heic: return "heic"
        case .tiff: return "tiff"
        case .webp: return "webp"
        }
    }

    public var utType: UTType {
        switch self {
        case .png: return .png
        case .jpeg: return .jpeg
        case .heic: return .heic
        case .tiff: return .tiff
        case .webp: return .webP
        }
    }

    /// Formats that keep every pixel exactly. JPEG/WebP here are lossy by nature;
    /// HEIC is lossy as CoreGraphics encodes it.
    public var isLossless: Bool {
        switch self {
        case .png, .tiff: return true
        case .jpeg, .heic, .webp: return false
        }
    }

    /// Whether a quality slider applies.
    public var supportsQuality: Bool { !isLossless }

    /// Whether this Mac can actually *write* the format. WebP encoding in
    /// particular is not available on every macOS version, so the UI hides
    /// options that would fail at save time.
    public var canEncode: Bool {
        (CGImageDestinationCopyTypeIdentifiers() as? [String])?
            .contains(utType.identifier) ?? false
    }

    public static var encodable: [ImageFormat] {
        allCases.filter(\.canEncode)
    }

    /// Extensions we accept as input. Broader than the encodable set because
    /// ImageIO reads more formats than it writes.
    public static let readableExtensions: Set<String> = [
        "heic", "heif", "png", "jpg", "jpeg", "tiff", "tif",
        "gif", "bmp", "webp", "avif", "dng", "cr2", "nef", "arw", "orf", "raf",
    ]

    public static func isReadable(_ url: URL) -> Bool {
        readableExtensions.contains(url.pathExtension.lowercased())
    }
}
