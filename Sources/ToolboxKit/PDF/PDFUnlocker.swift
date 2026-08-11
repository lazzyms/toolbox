import Foundation
import PDFKit

public enum PDFUnlocker {

    public struct Result: Sendable {
        public let output: URL
        public let pageCount: Int
    }

    /// True when the file is a PDF that needs a password to open.
    /// Used to give the user feedback before they commit to a run.
    public static func isEncrypted(_ url: URL) -> Bool {
        guard let doc = PDFDocument(url: url) else { return false }
        return doc.isLocked
    }

    /// Removes the open password from `input`, writing a decrypted copy.
    ///
    /// This requires the real password — it unlocks with the user's credential and
    /// re-serialises the already-authorised document. It does not crack anything.
    public static func removePassword(
        from input: URL,
        password: String,
        to location: OutputLocation = .alongsideInput
    ) throws -> Result {
        guard input.pathExtension.lowercased() == "pdf" else {
            throw ToolboxError.unsupportedInput(input.pathExtension)
        }
        // PDFDocument(url:) returns nil for unreadable/corrupt files but succeeds
        // (with isLocked == true) for encrypted ones.
        guard let doc = PDFDocument(url: input) else {
            throw ToolboxError.notAPDF(input)
        }

        if doc.isLocked {
            guard doc.unlock(withPassword: password) else {
                throw ToolboxError.wrongPassword
            }
        } else if !doc.isEncrypted {
            throw ToolboxError.notEncrypted
        }
        // An encrypted-but-not-locked doc (owner password only, empty user
        // password) still benefits from a rewrite, so fall through.

        let output = OutputNaming.destination(
            for: input, in: location, suffix: "-unlocked", extension: "pdf"
        )

        // PDFDocument.write(to:) on an unlocked document *keeps* the original
        // encryption dictionary — the result still prompts for a password. So
        // rebuild instead: copy the decrypted pages into a fresh document, which
        // has no encryption to inherit. Text stays selectable and searchable.
        let rebuilt = PDFDocument()
        for index in 0..<doc.pageCount {
            guard let page = doc.page(at: index)?.copy() as? PDFPage else { continue }
            rebuilt.insert(page, at: rebuilt.pageCount)
        }

        var wroteCleanCopy = rebuilt.pageCount == doc.pageCount
            && rebuilt.pageCount > 0
            && rebuilt.write(to: output)

        // Verify rather than trust: the copy must open with no password at all.
        if wroteCleanCopy, let check = PDFDocument(url: output), check.isLocked {
            wroteCleanCopy = false
        }

        if !wroteCleanCopy {
            // Fallback: re-render each page through CoreGraphics. Also drops the
            // encryption and keeps vector content, so text survives here too.
            try? FileManager.default.removeItem(at: output)
            try redrawWithCoreGraphics(input: input, password: password, output: output)
        }

        guard let check = PDFDocument(url: output), !check.isLocked, !check.isEncrypted else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.writeFailed(output)
        }

        return Result(output: output, pageCount: check.pageCount)
    }

    /// Re-renders every page into a new PDF via CoreGraphics, which never carries
    /// the source encryption across.
    private static func redrawWithCoreGraphics(
        input: URL, password: String, output: URL
    ) throws {
        guard let source = CGPDFDocument(input as CFURL) else {
            throw ToolboxError.notAPDF(input)
        }

        if source.isEncrypted, !source.isUnlocked {
            guard source.unlockWithPassword(password) else {
                throw ToolboxError.wrongPassword
            }
        }

        guard source.numberOfPages > 0,
              let firstPage = source.page(at: 1)
        else {
            throw ToolboxError.notAPDF(input)
        }

        var mediaBox = firstPage.getBoxRect(.mediaBox)
        guard let context = CGContext(output as CFURL, mediaBox: &mediaBox, nil) else {
            throw ToolboxError.writeFailed(output)
        }

        // CGPDFDocument pages are 1-indexed.
        for number in 1...source.numberOfPages {
            guard let page = source.page(at: number) else { continue }
            // Each page keeps its own size, so mixed-format documents survive.
            var box = page.getBoxRect(.mediaBox)
            context.beginPage(mediaBox: &box)
            context.drawPDFPage(page)
            context.endPage()
        }
        context.closePDF()
    }
}
