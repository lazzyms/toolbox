import SwiftUI

/// A single tool in the sidebar.
///
/// Adding a utility means adding one `Utility` value to `Utility.all` and one
/// SwiftUI view. Nothing else in the app needs to change.
struct Utility: Identifiable, Hashable {
    let id: String
    let title: String
    /// One or two words for the menu bar panel's picker chips, where `title`
    /// would truncate at half the popover's width.
    let shortTitle: String
    /// Shown under the title in the sidebar and at the top of the detail pane.
    let blurb: String
    let symbol: String
    let tint: Color
    let category: Category

    enum Category: String, CaseIterable, Identifiable {
        case pdf = "PDF"
        case images = "Images"

        var id: String { rawValue }
    }

    @ViewBuilder
    @MainActor
    func makeView() -> some View {
        switch id {
        case "pdf-unlock": PDFUnlockView(utility: self)
        case "heic-convert": ConvertView(utility: self)
        case "compress": CompressView(utility: self)
        case "resize": ResizeView(utility: self)
        default: UnavailableView(utility: self)
        }
    }

    static let all: [Utility] = [
        Utility(
            id: "pdf-unlock",
            title: "Remove PDF Password",
            shortTitle: "Unlock PDF",
            blurb: "Save an unlocked copy of a PDF you know the password for.",
            symbol: "lock.open.fill",
            tint: .orange,
            category: .pdf
        ),
        Utility(
            id: "heic-convert",
            title: "Convert Image Format",
            shortTitle: "Convert",
            blurb: "HEIC to PNG, JPEG and back — batch friendly.",
            symbol: "arrow.triangle.2.circlepath",
            tint: .blue,
            category: .images
        ),
        Utility(
            id: "compress",
            title: "Compress Images",
            shortTitle: "Compress",
            blurb: "Shrink files losslessly, or trade quality for size.",
            symbol: "arrow.down.circle.fill",
            tint: .green,
            category: .images
        ),
        Utility(
            id: "resize",
            title: "Resize Images",
            shortTitle: "Resize",
            blurb: "Scale by pixels, percentage or longest side.",
            symbol: "aspectratio.fill",
            tint: .purple,
            category: .images
        ),
    ]

    static func inCategory(_ category: Category) -> [Utility] {
        all.filter { $0.category == category }
    }
}

struct UnavailableView: View {
    let utility: Utility

    var body: some View {
        ContentUnavailableView(
            "Not implemented",
            systemImage: "hammer",
            description: Text("“\(utility.title)” has no view yet.")
        )
    }
}
