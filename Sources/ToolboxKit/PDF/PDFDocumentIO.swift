import Foundation
import PDFKit

/// Shared open / unlock / rebuild / save helpers for the PDF tools.
///
/// Every PDF tool starts with the same dance — validate the extension, open
/// or throw, unlock if a password is present — and must end through
/// `OutputNaming.destination` so nothing is ever overwritten. Keeping that in
/// one place is what stops each tool from getting a subtly different version.
public enum PDFDocumentIO {

    /// Opens `url` as a PDF, throwing a useful `ToolboxError` instead of
    /// returning nil. An encrypted file still opens — the caller decides what
    /// to do with `document.isLocked`.
    public static func open(_ url: URL) throws -> PDFDocument {
        guard url.pathExtension.lowercased() == "pdf" else {
            throw ToolboxError.unsupportedInput(url.pathExtension)
        }
        // PDFDocument(url:) returns nil for unreadable/corrupt files but
        // succeeds (with isLocked == true) for encrypted ones.
        guard let document = PDFDocument(url: url) else {
            throw ToolboxError.notAPDF(url)
        }
        return document
    }

    /// Unlocks `document` with `password`. A document that is already open is
    /// returned untouched, so callers can apply this unconditionally.
    @discardableResult
    public static func unlock(_ document: PDFDocument, password: String) throws -> PDFDocument {
        guard document.isLocked else { return document }
        guard document.unlock(withPassword: password) else {
            throw ToolboxError.wrongPassword
        }
        return document
    }

    /// A new `PDFDocument` holding copies of every page of `source`.
    ///
    /// `PDFDocument.write(to:)` on a document that came from an encrypted file
    /// *keeps* the source's encryption dictionary — the result still prompts
    /// for a password. Any tool that copies pages out of an unlocked document
    /// must rebuild into a fresh document before saving, which has no
    /// encryption to inherit.
    public static func copy(_ source: PDFDocument) -> PDFDocument {
        let copy = PDFDocument()
        for index in 0..<source.pageCount {
            guard let page = source.page(at: index)?.copy() as? PDFPage else { continue }
            copy.insert(page, at: copy.pageCount)
        }
        return copy
    }

    /// Writes `document` to `destination`, throwing `writeFailed` instead of
    /// returning false. Callers route `destination` through
    /// `OutputNaming.destination` first, so nothing is ever overwritten.
    public static func save(_ document: PDFDocument, to destination: URL) throws {
        guard document.write(to: destination) else {
            throw ToolboxError.writeFailed(destination)
        }
    }
}
