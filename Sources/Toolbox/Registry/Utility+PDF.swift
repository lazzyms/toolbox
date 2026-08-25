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
        Utility(id: "pdf-watermark", title: "Watermark PDF", shortTitle: "Watermark", blurb: "Stamp text or an image across pages — DRAFT, CONFIDENTIAL, a logo.", symbol: "paintbrush", tint: .cyan, category: .pdf, pane: { PDFWatermarkView(utility: $0) }),
        Utility(id: "pdf-crop", title: "Crop PDF", shortTitle: "Crop", blurb: "Trim margins or cut to a region — losslessly.", symbol: "crop.rotate", tint: .mint, category: .pdf, pane: { PDFCropView(utility: $0) }),
        Utility(id: "pdf-protect", title: "Protect PDF", shortTitle: "Protect", blurb: "Add a password so only you can open it.", symbol: "lock.fill", tint: .red, category: .pdf, pane: { PDFProtectView(utility: $0) }),
        Utility(id: "images-to-pdf", title: "Images to PDF", shortTitle: "Images→PDF", blurb: "Turn photos and scans into one PDF — HEIC included.", symbol: "photo.on.rectangle.angled", tint: .blue, category: .pdf, pane: { ImagesToPDFView(utility: $0) }),
        Utility(id: "pdf-to-images", title: "PDF to Images", shortTitle: "PDF→Images", blurb: "Render pages to JPEG or PNG at 72–300 dpi.", symbol: "photo.stack", tint: .indigo, category: .pdf, pane: { PDFToImagesView(utility: $0) }),
    ]
}
