import Foundation
import PDFKit

public enum PDFMerger {
    public static func merge(_ inputs: [URL], to location: OutputLocation) throws -> URL {
        guard !inputs.isEmpty else {
            throw ToolboxError.notAPDF(URL(fileURLWithPath: ""))
        }
        // Reject encrypted inputs
        for url in inputs {
            if PDFUnlocker.isEncrypted(url) {
                throw ToolboxError.passwordProtected(url)
            }
        }

        let merged = PDFDocument()
        for url in inputs {
            let doc = try PDFDocumentIO.open(url)
            for index in 0..<doc.pageCount {
                guard let page = doc.page(at: index)?.copy() as? PDFPage else { continue }
                merged.insert(page, at: merged.pageCount)
            }
        }

        guard merged.pageCount > 0 else {
            throw ToolboxError.notAPDF(inputs[0])
        }

        let output = OutputNaming.destination(
            for: inputs[0],
            in: location,
            suffix: "-merged",
            extension: "pdf"
        )
        try PDFDocumentIO.save(merged, to: output)
        return output
    }
}
