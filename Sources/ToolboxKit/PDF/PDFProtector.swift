import Foundation
import PDFKit

public enum PDFProtector {
    /// Adds password protection to a PDF and verifies the result actually
    /// locks. The worst failure mode here is silently writing an unencrypted
    /// copy, so the output is reopened and checked before it is handed back.
    public static func apply(
        password: String,
        ownerPassword: String?,
        to input: URL,
        to location: OutputLocation = .alongsideInput
    ) throws -> URL {
        if password.isEmpty {
            throw ToolboxError.emptyPassword
        }
        if PDFUnlocker.isEncrypted(input) {
            throw ToolboxError.passwordProtected(input)
        }

        let doc = try PDFDocumentIO.open(input)

        let output = OutputNaming.destination(
            for: input, in: location, suffix: "-protected", extension: "pdf"
        )

        // PDFKit only encrypts when an owner password is present, so fall
        // back to the user password when none was given (same requirement as
        // the fixture writer).
        let options: [PDFDocumentWriteOption: Any] = [
            .userPasswordOption: password,
            .ownerPasswordOption: ownerPassword ?? password,
        ]
        guard doc.write(to: output, withOptions: options) else {
            throw ToolboxError.writeFailed(output)
        }

        // Verify rather than trust: reopen, require the lock, require that the
        // empty password does not get in. Delete the copy if either check fails.
        guard let written = PDFDocument(url: output),
              written.isLocked,
              !written.unlock(withPassword: "") else {
            try? FileManager.default.removeItem(at: output)
            throw ToolboxError.protectionFailed(output)
        }

        return output
    }

    /// Names the encryption algorithm found in the file's /Encrypt dictionary,
    /// so the UI can state plainly what actually protects the document.
    /// Returns nil when no encryption marker can be found.
    public static func encryptionAlgorithm(of url: URL) -> String? {
        guard let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { return nil }
        // Scan only the tail: /Encrypt lives in the trailer region.
        let tail = data.suffix(4096)
        if range(of: aesv3, in: tail) != nil { return "AES-256" }
        if range(of: aesv2, in: tail) != nil { return "AES-128" }
        if range(of: v2Marker, in: tail) != nil { return "RC4 128-bit" }
        return nil
    }

    private static let aesv3 = Data("/AESV3".utf8)
    private static let aesv2 = Data("/AESV2".utf8)
    private static let v2Marker = Data("/V 2".utf8)

    private static func range(of needle: Data, in haystack: Data) -> Range<Data.Index>? {
        haystack.range(of: needle)
    }
}
