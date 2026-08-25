import Foundation
import PDFKit
import Testing
@testable import ToolboxKit

/// The organise tool is a plan applied to a document: which original pages
/// survive, in what order, turned how far. These pin the plan semantics and the
/// rebuild loop — order permutation, quarter-turn-only rotation baked into the
/// copies, omission as deletion — plus the guards (empty plan, bad angle,
/// encrypted input) that keep a bad grid from writing a bad file.
@Suite("Page Organizer")
struct PageOrganizerTests {

    // MARK: - Plan application

    @Test("pages come out exactly in the order the plan lists them")
    func reordersPagesExactlyAsPlanned() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "organize-order", pages: 3)

        let output = try PageOrganizer.apply(
            plan: OrganizePlan([.keep(index: 2), .keep(index: 0), .keep(index: 1)]),
            to: input
        )

        #expect(output.lastPathComponent == "organize-order-organized.pdf")
        // Each fixture page carries its own number as text, so string
        // extraction reads the permutation straight off the output.
        #expect(try pageStrings(of: output) == [
            "Toolbox test document 3",
            "Toolbox test document 1",
            "Toolbox test document 2",
        ])
    }

    @Test("rotation lands on the copied page, losslessly, and the input stays square")
    func rotationIsBakedWithoutTouchingTheInput() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "organize-turn", pages: 2)

        let output = try PageOrganizer.apply(
            plan: OrganizePlan([.rotate(index: 1, degrees: 90), .keep(index: 0)]),
            to: input
        )

        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 2)
        #expect(doc.page(at: 0)?.rotation == 90)
        #expect(doc.page(at: 1)?.rotation == 0)
        // Setting .rotation turns the page without rasterising it, so the text
        // underneath must still be extractable.
        #expect(doc.page(at: 0)?.string?.contains("2") == true)

        let original = try PDFDocumentIO.open(input)
        for index in 0..<original.pageCount {
            #expect(original.page(at: index)?.rotation == 0)
        }
        #expect(original.pageCount == 2)
    }

    @Test("reorder, rotate and delete compose in one pass")
    func onePassAppliesEveryOpKind() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "organize-combo", pages: 4)

        let output = try PageOrganizer.apply(
            plan: OrganizePlan([
                .rotate(index: 3, degrees: 180),
                .keep(index: 0),
                .rotate(index: 1, degrees: -90),
            ]),
            to: input
        )

        let doc = try PDFDocumentIO.open(output)
        #expect(doc.pageCount == 3)
        #expect(try pageStrings(of: output) == [
            "Toolbox test document 4",
            "Toolbox test document 1",
            "Toolbox test document 2",
        ])
        #expect(doc.page(at: 0)?.rotation == 180)
        #expect(doc.page(at: 1)?.rotation == 0)
        // A counter-clockwise turn folds into 270°, not -90°.
        #expect(doc.page(at: 2)?.rotation == 270)
    }

    // MARK: - Refusals

    @Test("an angle that isn't a whole quarter turn is refused, writing nothing")
    func arbitraryAngleRefused() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "organize-tilt", pages: 2)
        let before = try FileManager.default.contentsOfDirectory(atPath: fixtures.directory.path)

        #expect(throws: ToolboxError.unsupportedRotation(45)) {
            try PageOrganizer.apply(
                plan: OrganizePlan([.rotate(index: 0, degrees: 45)]),
                to: input
            )
        }

        #expect(try FileManager.default.contentsOfDirectory(atPath: fixtures.directory.path) == before)
    }

    @Test("a plan that keeps no pages at all is refused")
    func deletingEveryPageRefused() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "organize-empty", pages: 2)
        let before = try FileManager.default.contentsOfDirectory(atPath: fixtures.directory.path)

        #expect(throws: ToolboxError.emptyPlan) {
            try PageOrganizer.apply(plan: OrganizePlan([]), to: input)
        }

        #expect(throws: ToolboxError.emptyPlan) {
            try PageOrganizer.apply(plan: OrganizePlan.identity(pageCount: 0), to: input)
        }
        #expect(try FileManager.default.contentsOfDirectory(atPath: fixtures.directory.path) == before)
    }

    @Test("an index beyond the document is refused, naming the page")
    func indexBeyondTheDocumentRefused() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "organize-range", pages: 2)

        #expect(throws: ToolboxError.invalidPageRange(
            "Page 5 doesn't exist in a 2-page document."
        )) {
            try PageOrganizer.apply(
                plan: OrganizePlan([.keep(index: 0), .keep(index: 4)]),
                to: input
            )
        }
    }

    @Test("encrypted input is refused up front")
    func rejectsEncryptedInput() throws {
        let fixtures = try Fixtures()
        let locked = try fixtures.pdf(named: "organize-locked", password: "s3cret")

        #expect(throws: ToolboxError.passwordProtected(locked)) {
            try PageOrganizer.apply(
                plan: OrganizePlan([.keep(index: 0)]),
                to: locked
            )
        }
    }

    // MARK: - Naming

    @Test("running twice never overwrites the first result")
    func neverOverwritesAnExistingOutput() throws {
        let fixtures = try Fixtures()
        let input = try fixtures.pdf(named: "collide", pages: 2)
        let plan = OrganizePlan([.keep(index: 1), .keep(index: 0)])

        let first = try PageOrganizer.apply(plan: plan, to: input)
        let second = try PageOrganizer.apply(plan: plan, to: input)

        #expect(first.lastPathComponent == "collide-organized.pdf")
        #expect(second.lastPathComponent == "collide-organized-1.pdf")
        #expect(FileManager.default.fileExists(atPath: first.path))
    }

    // MARK: - Reading files back

    private func pageStrings(of url: URL) throws -> [String] {
        let doc = try PDFDocumentIO.open(url)
        return (0..<doc.pageCount).compactMap { doc.page(at: $0)?.string }
    }
}
