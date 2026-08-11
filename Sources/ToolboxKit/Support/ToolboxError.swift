import Foundation

public enum ToolboxError: LocalizedError, Equatable {
    case cannotOpen(URL)
    case notAPDF(URL)
    case wrongPassword
    case notEncrypted
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
        }
    }

    /// Shown under the message in the UI when there is something actionable.
    public var recoverySuggestion: String? {
        switch self {
        case .wrongPassword:
            return "Check the password and try again. It is case-sensitive."
        case .notEncrypted:
            return "You can open and save it normally — nothing to remove."
        case .noGain:
            return "The original file was copied unchanged."
        case .wouldDropFrames:
            return "Nothing was written. Resize and Compress keep every frame, "
                + "because they write the file back in its own format."
        case .cannotWriteFrames:
            return "Nothing was written and the original is untouched."
        case .unsupportedRotation:
            return "Rotation works in quarter turns: 90°, 180° or 270°."
        default:
            return nil
        }
    }
}
