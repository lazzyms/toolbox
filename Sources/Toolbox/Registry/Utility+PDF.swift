import SwiftUI

extension Utility {
    /// The PDF tools, in the order they appear in the sidebar.
    ///
    /// One entry per line, deliberately past the usual column budget: adding a
    /// tool is then a single appended line, and two branches that both add one
    /// conflict on that line alone — resolved by keeping both.
    static let pdfTools: [Utility] = [
        Utility(id: "pdf-unlock", title: "Remove PDF Password", shortTitle: "Unlock PDF", blurb: "Save an unlocked copy of a PDF you know the password for.", symbol: "lock.open.fill", tint: .orange, category: .pdf, pane: { PDFUnlockView(utility: $0) }),
        Utility(id: "pdf-page-numbers", title: "Add PDF Page Numbers", shortTitle: "Page Numbers", blurb: "Stamp page numbers onto a PDF.", symbol: "number", tint: .blue, category: .pdf, pane: { PDFPageNumbersView(utility: $0) }),
        Utility(id: "pdf-merge", title: "Merge PDF", shortTitle: "Merge", blurb: "Combine several PDFs into one file.", symbol: "square.on.square", tint: .purple, category: .pdf, pane: { PDFMergeView(utility: $0) }),
    ]
}
