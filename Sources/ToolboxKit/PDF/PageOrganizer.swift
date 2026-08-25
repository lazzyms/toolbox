import Foundation
import PDFKit

/// One step in an organise plan: an original page, where it lands in the
/// output, and how it is turned.
///
/// Indices refer to the input document's zero-based pages. A page that should
/// not appear in the output simply has no op — omission is deletion.
public enum OrganizeOp: Equatable, Sendable {
    /// Copy the page through unchanged.
    case keep(index: Int)
    /// Copy the page turned by `degrees` (a multiple of 90).
    case rotate(index: Int, degrees: Int)

    /// The original page this op draws from.
    public var index: Int {
        switch self {
        case .keep(let index): return index
        case .rotate(let index, _): return index
        }
    }
}

/// An ordered list of ops describing the output document, one op per output
/// page. Pure data — the view builds it from its thumbnail grid and the kit
/// applies it — so plan semantics stay testable without SwiftUI.
public struct OrganizePlan: Equatable, Sendable {
    public var ops: [OrganizeOp]

    public init(_ ops: [OrganizeOp]) {
        self.ops = ops
    }

    /// The plan that copies every page of a `pageCount`-page document as-is.
    public static func identity(pageCount: Int) -> OrganizePlan {
        OrganizePlan((0..<max(0, pageCount)).map { .keep(index: $0) })
    }

    /// Checks a plan against a concrete document before anything is written:
    /// at least one page must survive, every referenced page must exist, and
    /// rotations must be whole quarter turns (PDFKit raises on anything else,
    /// so refusing here is what keeps a bad grid from crashing the app).
    public func validate(pageCount: Int) throws {
        guard !ops.isEmpty else { throw ToolboxError.emptyPlan }
        for op in ops {
            guard op.index >= 0, op.index < pageCount else {
                throw ToolboxError.invalidPageRange(
                    "Page \(op.index + 1) doesn't exist in a \(pageCount)-page document."
                )
            }
            if case .rotate(_, let degrees) = op {
                guard degrees % 90 == 0 else {
                    throw ToolboxError.unsupportedRotation(degrees)
                }
            }
        }
    }
}

/// Reorders, rotates and deletes pages of one PDF by rebuilding a fresh
/// document in plan order — the same fresh-copy loop the other PDF tools use,
/// driven by the plan instead of "every page, front to back".
public enum PageOrganizer {

    /// Applies `plan` to the document at `input`, writing a "-organized" copy.
    ///
    /// Rotation is set on the copied pages via `PDFPage.rotation`, which is
    /// lossless: the content stream is untouched and viewers re-render the
    /// turn. The input file is never modified.
    @discardableResult
    public static func apply(
        plan: OrganizePlan,
        to input: URL,
        location: OutputLocation = .alongsideInput
    ) throws -> URL {
        // An encrypted source would copy locked pages into a document that
        // still prompts for a password; refuse up front like Merge does.
        if PDFUnlocker.isEncrypted(input) {
            throw ToolboxError.passwordProtected(input)
        }

        let document = try PDFDocumentIO.open(input)
        try plan.validate(pageCount: document.pageCount)

        var organised = PDFDocument()
        for op in plan.ops {
            guard let copied = document.page(at: op.index)?.copy() as? PDFPage else {
                throw ToolboxError.notAPDF(input)
            }
            if case .rotate(_, let degrees) = op {
                copied.rotation = normalised(copied.rotation + degrees)
            }
            organised.insert(copied, at: organised.pageCount)
        }

        let output = OutputNaming.destination(
            for: input, in: location, suffix: "-organized", extension: "pdf"
        )
        try PDFDocumentIO.save(organised, to: output)
        return output
    }

    /// Folds accumulated turns back into 0/90/180/270.
    private static func normalised(_ degrees: Int) -> Int {
        ((degrees % 360) + 360) % 360
    }
}
