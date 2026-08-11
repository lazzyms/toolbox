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
        default:
            return nil
        }
    }
}
