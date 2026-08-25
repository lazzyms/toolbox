import Foundation

public enum ToolboxError: LocalizedError, Equatable {
    case cannotOpen(URL)
    case notAPDF(URL)
    case wrongPassword
    case notEncrypted
    /// An operation that can't read through a password met one.
    case passwordProtected(URL)
    case unsupportedInput(String)
    case unsupportedOutput(String)
    case decodeFailed(URL)
    case encodeFailed(String)
    case writeFailed(URL)
    case invalidDimensions
    case noGain
    /// The output format can hold one image, and the input has several.
    case wouldDropFrames(URL, frames: Int, format: String)
    /// The frames read fine, but ImageIO can't write that format's animation.
    case cannotWriteFrames(URL, frames: Int, format: String)
    /// A rotation that isn't a whole quarter turn.
    case unsupportedRotation(Int)
    /// A page range that can't be applied, carrying the reason as a message.
    case invalidPageRange(String)
    /// An icon set with no usable sizes, carrying the reason as a message.
    case invalidIconSizes(String)
    /// A GIF-tool request that can't be honoured, carrying the reason.
    case invalidGIFOptions(String)
    /// A "split the animation" request landed on something with a single frame.
    case notAnimated(URL)
    /// A crop that can't be applied, carrying the reason as a message.
    case invalidCrop(String)
    /// A watermark request with neither usable text nor an image.
    case emptyWatermark
    /// A PDF crop box that is empty or falls outside the page.
    case invalidCropBox(String)
    /// A protect request with no password.
    case emptyPassword
    /// A protection write that couldn't be verified as locked.
    case protectionFailed(URL)
    /// A job that needs files was handed none.
    case emptySelection
    /// A render whose pixel dimensions would exhaust memory, carrying the size.
    case resolutionTooLarge(Int, Int)
    /// A text extraction that found no text layer — the file is a scan.
    case noTextLayer(URL)
    /// A split request that can't be honoured, carrying the reason.
    case invalidSplit(String)
    /// A sign request with neither a readable signature image nor a typed name.
    case emptySignature
    /// The Vision OCR engine itself failed, carrying its message.
    case ocrFailed(String)

    public var errorDescription: String? {
        switch self {
        case .cannotOpen(let url):
            return "Couldn't open “\(url.lastPathComponent)”."
        case .notAPDF(let url):
            return "“\(url.lastPathComponent)” isn't a readable PDF."
        case .wrongPassword:
            return "Incorrect password."
        case .notEncrypted:
            return "This PDF has no password."
        case .passwordProtected(let url):
            return "“\(url.lastPathComponent)” is password-protected."
        case .unsupportedInput(let ext):
            return "“\(ext)” files aren't supported here."
        case .unsupportedOutput(let name):
            return "This Mac can't write \(name) files."
        case .decodeFailed(let url):
            return "Couldn't decode the image data in “\(url.lastPathComponent)”."
        case .encodeFailed(let name):
            return "Failed to encode \(name) data."
        case .writeFailed(let url):
            return "Couldn't write to “\(url.lastPathComponent)”."
        case .invalidDimensions:
            return "Enter a width or a height greater than zero."
        case .noGain:
            return "Already optimized — the original was smaller."
        case .wouldDropFrames(let url, let frames, let format):
            return "“\(url.lastPathComponent)” has \(frames) frames, and a \(format) "
                + "file can only hold the first one."
        case .cannotWriteFrames(let url, let frames, let format):
            return "“\(url.lastPathComponent)” has \(frames) frames, and this Mac "
                + "can't write an animated \(format) file."
        case .unsupportedRotation(let degrees):
            return "Can't rotate by \(degrees)°."
        case .invalidPageRange(let reason):
            return reason
        case .invalidIconSizes(let reason):
            return reason
        case .invalidGIFOptions(let reason):
            return reason
        case .notAnimated(let url):
            return "“\(url.lastPathComponent)” has a single frame — nothing to extract."
        case .invalidCrop(let reason):
            return reason
        case .emptyWatermark:
            return "Enter watermark text or choose an image."
        case .invalidCropBox(let reason):
            return reason
        case .emptyPassword:
            return "Enter a password."
        case .protectionFailed:
            return "Protection couldn't be verified — no file was written."
        case .emptySelection:
            return "Add at least one file."
        case .resolutionTooLarge(let width, let height):
            return "That would render \(width)×\(height) pixels — pick a lower DPI."
        case .noTextLayer(let url):
            return "“\(url.lastPathComponent)” has no selectable text — this looks like a scan."
        case .invalidSplit(let reason):
            return reason
        case .emptySignature:
            return "Choose a signature image or type a name to sign with."
        case .ocrFailed(let reason):
            return "Text recognition failed: \(reason)"
        }
    }

    /// Shown under the message in the UI when there is something actionable.
    public var recoverySuggestion: String? {
        switch self {
        case .wrongPassword:
            return "Check the password and try again. It is case-sensitive."
        case .notEncrypted:
            return "You can open and save it normally — nothing to remove."
        case .passwordProtected:
            return "Unlock it first, then run the tool again."
        case .protectionFailed:
            return "The original is untouched."
        case .noGain:
            return "The original file was copied unchanged."
        case .wouldDropFrames:
            return "Nothing was written. Resize and Compress keep every frame, "
                + "because they write the file back in its own format."
        case .cannotWriteFrames:
            return "Nothing was written and the original is untouched."
        case .unsupportedRotation:
            return "Rotation works in quarter turns: 90°, 180° or 270°."
        case .noTextLayer:
            return "Try the OCR tool to read text out of a scan."
        default:
            return nil
        }
    }
}
