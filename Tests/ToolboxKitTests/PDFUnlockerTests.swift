import Foundation
import PDFKit
import Testing
@testable import ToolboxKit

@Suite("PDFUnlocker")
struct PDFUnlockerTests {

    @Test("removes the password with the correct credential")
    func removesPassword() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(
            named: "statement", password: "s3cret", text: "ACCOUNT_SUMMARY"
        )
        try #require(PDFUnlocker.isEncrypted(locked))

        let result = try PDFUnlocker.removePassword(
            from: locked, password: "s3cret", to: .directory(fixtures.directory)
        )

        // The real assertion: the copy opens with no password at all.
        let reopened = try #require(PDFDocument(url: result.output))
        #expect(reopened.isLocked == false)
        #expect(reopened.isEncrypted == false)
        #expect(result.pageCount == 1)
    }

    @Test("keeps text selectable instead of rasterising")
    func preservesText() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(
            named: "contract", password: "pw", text: "SEARCHABLE_CLAUSE"
        )

        let result = try PDFUnlocker.removePassword(
            from: locked, password: "pw", to: .directory(fixtures.directory)
        )

        let text = try #require(PDFDocument(url: result.output)?.string)
        #expect(text.contains("SEARCHABLE_CLAUSE"))
    }

    @Test("preserves every page of a multi-page document")
    func preservesAllPages() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "report", password: "pw", pages: 5)

        let result = try PDFUnlocker.removePassword(
            from: locked, password: "pw", to: .directory(fixtures.directory)
        )

        #expect(result.pageCount == 5)
        #expect(PDFDocument(url: result.output)?.pageCount == 5)
    }

    @Test("rejects a wrong password")
    func wrongPassword() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "locked", password: "correct")

        #expect(throws: ToolboxError.wrongPassword) {
            try PDFUnlocker.removePassword(
                from: locked, password: "guess", to: .directory(fixtures.directory)
            )
        }
    }

    @Test("leaves no output file behind when the password fails")
    func noPartialOutput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "clean", password: "pw")

        _ = try? PDFUnlocker.removePassword(
            from: locked, password: "wrong", to: .directory(fixtures.directory)
        )

        let contents = try FileManager.default.contentsOfDirectory(
            atPath: fixtures.directory.path
        )
        #expect(contents.filter { $0.contains("unlocked") }.isEmpty)
    }

    @Test("reports when a PDF has no password")
    func notEncrypted() throws {
        let fixtures = try Fixtures()
        let plain = try fixtures.pdf(named: "open")

        #expect(throws: ToolboxError.notEncrypted) {
            try PDFUnlocker.removePassword(
                from: plain, password: "anything", to: .directory(fixtures.directory)
            )
        }
    }

    @Test("rejects non-PDF input")
    func rejectsNonPDF() throws {
        let fixtures = try Fixtures()
        let text = fixtures.directory.appendingPathComponent("note.txt")
        try "not a pdf".write(to: text, atomically: true, encoding: .utf8)

        #expect(throws: ToolboxError.unsupportedInput("txt")) {
            try PDFUnlocker.removePassword(from: text, password: "x")
        }
    }

    @Test("reports corrupt PDFs clearly")
    func corruptPDF() throws {
        let fixtures = try Fixtures()
        let corrupt = fixtures.directory.appendingPathComponent("bad.pdf")
        try Data("%PDF-1.4 truncated".utf8).write(to: corrupt)

        #expect(throws: ToolboxError.notAPDF(corrupt)) {
            try PDFUnlocker.removePassword(from: corrupt, password: "x")
        }
    }

    @Test("isEncrypted is false for plain and missing files")
    func isEncryptedNegatives() throws {
        let fixtures = try Fixtures()
        let plain = try fixtures.pdf(named: "plain")
        #expect(PDFUnlocker.isEncrypted(plain) == false)
        #expect(PDFUnlocker.isEncrypted(
            fixtures.directory.appendingPathComponent("ghost.pdf")
        ) == false)
    }
}

@Suite("BatchRunner")
struct BatchRunnerTests {

    @Test("one failure does not stop the batch")
    func isolatesFailures() async throws {
        let fixtures = try Fixtures()
        let good = try fixtures.pdf(named: "a", password: "pw")
        let bad = fixtures.directory.appendingPathComponent("missing.pdf")

        let outcomes = await BatchRunner.run([good, bad]) { url in
            let result = try PDFUnlocker.removePassword(
                from: url, password: "pw", to: .directory(fixtures.directory)
            )
            return JobOutcome(input: url, output: result.output, detail: "ok")
        }

        #expect(outcomes.count == 2)
        #expect(outcomes.filter(\.succeeded).count == 1)
        #expect(outcomes.first { !$0.succeeded }?.failure?.isEmpty == false)
    }

    @Test("preserves input order regardless of completion order")
    func preservesOrder() async throws {
        let inputs = (0..<24).map { URL(fileURLWithPath: "/tmp/file-\($0).png") }

        let outcomes = await BatchRunner.run(inputs) { url in
            JobOutcome(input: url, output: nil, detail: "")
        }

        #expect(outcomes.map(\.input) == inputs)
    }

    @Test("reports progress up to the total")
    func reportsProgress() async throws {
        let inputs = (0..<10).map { URL(fileURLWithPath: "/tmp/p-\($0).png") }
        let counter = Counter()

        _ = await BatchRunner.run(inputs, progress: { done, total in
            counter.record(done: done, total: total)
        }) { url in
            JobOutcome(input: url, output: nil, detail: "")
        }

        #expect(counter.maxDone == 10)
        #expect(counter.total == 10)
    }

    @Test("empty input returns no outcomes")
    func emptyInput() async {
        let outcomes = await BatchRunner.run([]) { url in
            JobOutcome(input: url, output: nil, detail: "")
        }
        #expect(outcomes.isEmpty)
    }
}

/// Thread-safe progress recorder for the batch tests.
private final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var _maxDone = 0
    private var _total = 0

    var maxDone: Int { lock.withLock { _maxDone } }
    var total: Int { lock.withLock { _total } }

    func record(done: Int, total: Int) {
        lock.withLock {
            _maxDone = max(_maxDone, done)
            _total = total
        }
    }
}
