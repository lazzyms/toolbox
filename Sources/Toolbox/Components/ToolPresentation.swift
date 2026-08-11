import SwiftUI

/// Where a tool's UI is currently being shown.
///
/// The views in `Features/` are used verbatim in the main window and in the menu
/// bar panel — this is what lets the shared chrome tighten its spacing for a
/// 380pt popover instead of forking every pane into a "compact" twin. Read it
/// from the environment, never construct one in a feature view.
enum ToolPresentation {
    /// The `NavigationSplitView` in the main window.
    case window
    /// The `MenuBarExtra` panel, where vertical space is the scarce resource.
    case menuBar

    var isCompact: Bool { self == .menuBar }

    /// Outer padding around a tool's body.
    var padding: CGFloat { isCompact ? 14 : 20 }

    /// Gap between the option rows and the drop zone.
    var contentSpacing: CGFloat { isCompact ? 12 : 18 }

    /// Height of the drop target. Small enough in the panel to leave room for
    /// the file queue below it without scrolling.
    var dropZonePadding: CGFloat { isCompact ? 18 : 30 }

    var dropZoneIconSize: CGFloat { isCompact ? 24 : 34 }

    /// Width of the leading label in an `OptionRow`.
    var optionLabelWidth: CGFloat { isCompact ? 68 : 92 }

    var fileListMaxHeight: CGFloat { isCompact ? 108 : 150 }

    var resultsMaxHeight: CGFloat { isCompact ? 120 : 180 }

    /// Caps the panel's scrolling area so the popover can't grow taller than the
    /// screen once a long file queue is added. Unbounded in the window, which
    /// resizes instead.
    var bodyMaxHeight: CGFloat? { isCompact ? 480 : nil }

    var runButtonSize: ControlSize { isCompact ? .regular : .large }

    /// Lines explanatory text up under an `OptionRow`'s content column.
    var explanationInset: CGFloat { optionLabelWidth + 8 }
}

extension View {
    /// Segmented in the window, a pop-up menu in the panel.
    ///
    /// Segments size to their labels and ignore the parent's width, so the four
    /// of Resize's "Method" row are ~470pt — they'd punch straight out of a
    /// 380pt popover. A pop-up is as wide as one label.
    @ViewBuilder
    func optionPickerStyle(_ presentation: ToolPresentation) -> some View {
        if presentation.isCompact {
            pickerStyle(.menu).fixedSize()
        } else {
            pickerStyle(.segmented).fixedSize()
        }
    }
}

private struct ToolPresentationKey: EnvironmentKey {
    static let defaultValue = ToolPresentation.window
}

extension EnvironmentValues {
    var toolPresentation: ToolPresentation {
        get { self[ToolPresentationKey.self] }
        set { self[ToolPresentationKey.self] = newValue }
    }
}
