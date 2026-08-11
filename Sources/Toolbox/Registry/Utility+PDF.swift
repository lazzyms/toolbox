import SwiftUI

extension Utility {
    /// The PDF tools, in the order they appear in the sidebar.
    ///
    /// One entry per line, deliberately past the usual column budget: adding a
    /// tool is then a single appended line, and two branches that both add one
    /// conflict on that line alone — resolved by keeping both.
    static let pdfTools: [Utility] = [
        Utility(id: "pdf-unlock", title: "Remove PDF Password", shortTitle: "Unlock PDF", blurb: "Save an unlocked copy of a PDF you know the password for.", symbol: "lock.open.fill", tint: .orange, category: .pdf, pane: { PDFUnlockView(utility: $0) }),
    ]
}
