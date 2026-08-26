import Testing
import Foundation
import PDFKit
@testable import ToolboxKit

@Suite("PDF Protector")
struct PDFProtectorTests {
    @Test func outputIsLockedAndRejectsEmptyPassword() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "protect-doc", text: "Secret body")

        let output = try PDFProtector.apply(
            password: "correct horse",
            ownerPassword: nil,
            to: input,
            to: .alongsideInput
        )

        #expect(output.lastPathComponent == "protect-doc-protected.pdf")

        let reopened = try #require(PDFDocument(url: output))
        #expect(reopened.isLocked)
        #expect(!reopened.unlock(withPassword: ""))
        #expect(reopened.unlock(withPassword: "correct horse"))
    }

    @Test func roundTripsWithTheUnlockTool() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "protect-roundtrip", text: "Selectable secret")

        let protected = try PDFProtector.apply(
            password: "pw123", ownerPassword: nil, to: input, to: .alongsideInput
        )
        let unlocked = try PDFUnlocker.removePassword(from: protected, password: "pw123")

        let doc = try PDFDocumentIO.open(unlocked.output)
        #expect(!doc.isLocked)
        #expect((doc.page(at: 0)?.string ?? "").contains("Selectable secret"))
    }

    @Test func statesTheAlgorithmActuallyUsed() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "protect-algo", text: "Body")

        let output = try PDFProtector.apply(
            password: "pw", ownerPassword: nil, to: input, to: .alongsideInput
        )

        // Documents what this macOS actually applies. If a future OS drops
        // AES this test fails on purpose, forcing the UI copy to be revisited.
        let algorithm = try #require(PDFProtector.encryptionAlgorithm(of: output))
        #expect(algorithm.contains("AES"))
    }

    @Test func rejectsAlreadyEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "protect-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PDFProtector.apply(password: "new", ownerPassword: nil, to: locked, to: .alongsideInput)
        }
    }

    @Test func rejectsEmptyPassword() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "protect-empty", text: "Body")

        #expect(throws: ToolboxError.emptyPassword) {
            try PDFProtector.apply(password: "", ownerPassword: nil, to: input, to: .alongsideInput)
        }
    }

    @Test func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "protect-collide", text: "One")

        let first = try PDFProtector.apply(password: "a", ownerPassword: nil, to: input, to: .alongsideInput)
        let second = try PDFProtector.apply(password: "a", ownerPassword: nil, to: input, to: .alongsideInput)

        #expect(first.lastPathComponent == "protect-collide-protected.pdf")
        #expect(second.lastPathComponent == "protect-collide-protected-1.pdf")
    }
}
