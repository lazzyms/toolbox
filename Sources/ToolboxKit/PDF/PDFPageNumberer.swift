import Foundation
import PDFKit
import CoreGraphics

public enum PDFPageNumberer {
    public enum Position: Sendable {
        case topLeft, topCenter, topRight
        case bottomLeft, bottomCenter, bottomRight
    }

    public enum Format: Sendable {
        case plain
        case pageOfTotal
        case pagePrefix
    }

    public static func addNumbers(
        to input: URL,
        position: Position,
        startNumber: Int,
        pageRangeText: String?,
        format: Format,
        fontSize: CGFloat,
        margin: CGFloat,
        to location: OutputLocation
    ) throws -> URL {
        let doc = try PDFDocumentIO.open(input)
        let pageCount = doc.pageCount
        guard pageCount > 0 else {
            throw ToolboxError.notAPDF(input)
        }

        let pagesToNumber: [Int]
        if let rangeText = pageRangeText, !rangeText.trimmingCharacters(in: .whitespaces).isEmpty {
            pagesToNumber = try PageRange.parse(rangeText, pageCount: pageCount)
        } else {
            pagesToNumber = Array(0..<pageCount)
        }

        let output = OutputNaming.destination(for: input, in: location, suffix: "-numbered", extension: "pdf")

        let firstPage = doc.page(at: 0)!
        var mediaBox = firstPage.bounds(for: .mediaBox)
        guard let context = CGContext(output as CFURL, mediaBox: &mediaBox, nil) else {
            throw ToolboxError.writeFailed(output)
        }

        for index in 0..<pageCount {
            guard let page = doc.page(at: index) else { continue }
            var box = page.bounds(for: .mediaBox)
            context.beginPage(mediaBox: &box)
            PDFPageReplay.replay(page: page, into: context)

            if pagesToNumber.contains(index) {
                let number = startNumber + index
                let total = pageCount
                let text = textForFormat(format, number: number, total: total)
                let stamp = TextStamp(
                    text: text,
                    fontName: "Helvetica",
                    size: .points(fontSize),
                    color: CGColor.black,
                    opacity: 1.0,
                    rotationDegrees: 0
                )
                let anchor = anchorForPosition(position)
                GraphicsStamp.draw(stamp, in: context, bounds: box, anchor: anchor, inset: margin)
            }
            context.endPage()
        }
        context.closePDF()
        return output
    }

    private static func textForFormat(_ format: Format, number: Int, total: Int) -> String {
        switch format {
        case .plain:
            return "\(number)"
        case .pageOfTotal:
            return "\(number) of \(total)"
        case .pagePrefix:
            return "Page \(number)"
        }
    }

    private static func anchorForPosition(_ position: Position) -> StampAnchor {
        switch position {
        case .topLeft: return .topLeft
        case .topCenter: return .top
        case .topRight: return .topRight
        case .bottomLeft: return .bottomLeft
        case .bottomCenter: return .bottom
        case .bottomRight: return .bottomRight
        }
    }
}
