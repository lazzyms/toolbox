import SwiftUI

extension Utility {
    /// The image tools, in the order they appear in the sidebar.
    ///
    /// One entry per line, deliberately past the usual column budget: adding a
    /// tool is then a single appended line, and two branches that both add one
    /// conflict on that line alone — resolved by keeping both.
    static let imageTools: [Utility] = [
        Utility(id: "heic-convert", title: "Convert Image Format", shortTitle: "Convert", blurb: "HEIC to PNG, JPEG and back — batch friendly.", symbol: "arrow.triangle.2.circlepath", tint: .blue, category: .images, pane: { ConvertView(utility: $0) }),
        Utility(id: "compress", title: "Compress Images", shortTitle: "Compress", blurb: "Shrink files losslessly, or trade quality for size.", symbol: "arrow.down.circle.fill", tint: .green, category: .images, pane: { CompressView(utility: $0) }),
        Utility(id: "resize", title: "Resize Images", shortTitle: "Resize", blurb: "Scale by pixels, percentage or longest side.", symbol: "aspectratio.fill", tint: .purple, category: .images, pane: { ResizeView(utility: $0) }),
    ]
}
