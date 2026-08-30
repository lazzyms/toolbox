import SwiftUI

/// A single tool in the sidebar.
///
/// A utility is declared in exactly one place — one line in its category's
/// registry (`Utility+PDF.swift`, `Utility+Images.swift`) — and carries the
/// factory for its own detail pane. There is no id-to-view lookup table to keep
/// in sync, so a tool with no view fails to compile instead of silently
/// rendering a placeholder at runtime.
struct Utility: Identifiable, Hashable {
    let id: String
    let title: String
    /// One or two words for the menu bar panel's picker chips, where `title`
    /// would truncate at half the popover's width.
    let shortTitle: String
    /// Shown under the title in the sidebar and at the top of the detail pane.
    let blurb: String
    /// The bundled Tabler icon name, without its `.svg` extension.
    let symbol: String
    let tint: Color
    let category: Category

    /// Builds this tool's detail pane.
    ///
    /// It takes the utility rather than closing over it because every pane hands
    /// one to `ToolScaffold` for its header, and a closure written inline in the
    /// registry can't refer to the value being initialised. `makeView()` feeds
    /// `self` back in, so registry entries stay one-liners.
    private let pane: @MainActor @Sendable (Utility) -> AnyView

    /// The pane is erased to `AnyView` on the way in: one is built when the
    /// selection changes and never in a hot path, so the erasure costs nothing
    /// measurable and buys a view factory that can live in a value.
    init<Pane: View>(
        id: String,
        title: String,
        shortTitle: String,
        blurb: String,
        symbol: String,
        tint: Color,
        category: Category,
        pane: @escaping @MainActor @Sendable (Utility) -> Pane
    ) {
        self.id = id
        self.title = title
        self.shortTitle = shortTitle
        self.blurb = blurb
        self.symbol = symbol
        self.tint = tint
        self.category = category
        self.pane = { AnyView(pane($0)) }
    }

    @MainActor
    func makeView() -> AnyView {
        pane(self)
    }

    enum Category: String, CaseIterable, Identifiable {
        case pdf = "PDF"
        case images = "Images"

        var id: String { rawValue }
    }

    /// Sidebar and picker order: categories as listed by `Category.allCases`,
    /// tools in the order of their own category's array.
    static let all: [Utility] = pdfTools + imageTools

    static func inCategory(_ category: Category) -> [Utility] {
        all.filter { $0.category == category }
    }

    // A stored closure isn't equatable, so neither conformance can be
    // synthesised. `id` is what identifies a tool everywhere else — the
    // selection binding, the detail pane's `.id()` keying — so key both on it.
    static func == (lhs: Utility, rhs: Utility) -> Bool {
        lhs.id == rhs.id
    }

    func hash(into hasher: inout Hasher) {
        hasher.combine(id)
    }
}
