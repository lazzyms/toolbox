import CoreGraphics
import PDFKit

public enum PDFPageReplay {
    public static func replay(
        page: PDFPage,
        into context: CGContext
    ) {
        context.saveGState()
        defer { context.restoreGState() }
        let rect = page.bounds(for: .mediaBox)
        context.translateBy(x: 0, y: rect.height)
        context.scaleBy(x: 1, y: -1)
        page.draw(with: .mediaBox, to: context)
    }
}
