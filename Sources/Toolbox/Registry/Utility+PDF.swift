import SwiftUI

extension Utility {
    /// The PDF tools, in the order they appear in the sidebar.
    ///
    /// One entry per line, deliberately past the usual column budget: adding a
    /// tool is then a single appended line, and two branches that both add one
    /// conflict on that line alone — resolved by keeping both.
    static let pdfTools: [Utility] = [
        Utility(id: "pdf-unlock", title: "Remove PDF Password", shortTitle: "Unlock PDF", blurb: "Save an unlocked copy of a PDF you know the password for.", category: .pdf, pane: { PDFUnlockView(utility: $0) }),
        Utility(id: "pdf-page-numbers", title: "Add PDF Page Numbers", shortTitle: "Page Numbers", blurb: "Stamp page numbers onto a PDF.", category: .pdf, pane: { PDFPageNumbersView(utility: $0) }),
        Utility(id: "pdf-merge", title: "Merge PDF", shortTitle: "Merge", blurb: "Combine several PDFs into one file.", category: .pdf, pane: { PDFMergeView(utility: $0) }),
        Utility(id: "pdf-watermark", title: "Watermark PDF", shortTitle: "Watermark", blurb: "Stamp text or an image across pages — DRAFT, CONFIDENTIAL, a logo.", category: .pdf, pane: { PDFWatermarkView(utility: $0) }),
        Utility(id: "pdf-crop", title: "Crop PDF", shortTitle: "Crop", blurb: "Trim margins or cut to a region — losslessly.", category: .pdf, pane: { PDFCropView(utility: $0) }),
        Utility(id: "pdf-protect", title: "Protect PDF", shortTitle: "Protect", blurb: "Add a password so only you can open it.", category: .pdf, pane: { PDFProtectView(utility: $0) }),
        Utility(id: "images-to-pdf", title: "Images to PDF", shortTitle: "Images→PDF", blurb: "Turn photos and scans into one PDF — HEIC included.", category: .pdf, pane: { ImagesToPDFView(utility: $0) }),
        Utility(id: "pdf-to-images", title: "PDF to Images", shortTitle: "PDF→Images", blurb: "Render pages to JPEG or PNG at 72–300 dpi.", category: .pdf, pane: { PDFToImagesView(utility: $0) }),
        Utility(id: "pdf-to-text", title: "PDF to Text", shortTitle: "PDF→Text", blurb: "Pull the text out as .txt or best-effort Markdown.", category: .pdf, pane: { PDFToTextView(utility: $0) }),
        Utility(id: "pdf-split", title: "Split PDF", shortTitle: "Split", blurb: "Break one PDF into several — every page, by ranges, or fixed-size chunks.", category: .pdf, pane: { PDFSplitView(utility: $0) }),
        Utility(id: "pdf-image-extract", title: "Extract Images from PDF", shortTitle: "Extract Images", blurb: "Pull embedded pictures out at their original resolution — JPEGs stay untouched.", category: .pdf, pane: { PDFImageExtractView(utility: $0) }),
        Utility(id: "pdf-sign", title: "Sign PDF", shortTitle: "Sign", blurb: "Stamp your signature image or typed name onto pages — visual only, not cryptographic.", category: .pdf, pane: { PDFSignView(utility: $0) }),
        Utility(id: "pdf-ocr", title: "OCR PDF", shortTitle: "OCR", blurb: "Read text out of scans with on-device OCR — outputs a .txt file.", category: .pdf, pane: { PDFOCRView(utility: $0) }),
        Utility(id: "pdf-remove-pages", title: "Remove PDF Pages", shortTitle: "Remove Pages", blurb: "Delete selected pages from a PDF — the rest stay, in order.", category: .pdf, pane: { PDFRemovePagesView(utility: $0) }),
        Utility(id: "pdf-extract-pages", title: "Extract PDF Pages", shortTitle: "Extract Pages", blurb: "Pull selected pages out into a new PDF — ranges like 1-3, 7.", category: .pdf, pane: { PageExtractView(utility: $0) }),
        Utility(id: "pdf-organize", title: "Organize PDF", shortTitle: "Organize", blurb: "Reorder, rotate and delete pages of one PDF from a thumbnail grid.", category: .pdf, pane: { OrganizeView(utility: $0) }),
        Utility(id: "pdf-compress", title: "Compress PDF", shortTitle: "Compress", blurb: "Shrink scan-heavy PDFs by rasterising pages as JPEG — text becomes pixels.", category: .pdf, pane: { PDFCompressView(utility: $0) }),
    ]
}
