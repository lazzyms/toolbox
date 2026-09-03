import AppKit
import SwiftUI

/// The full toolbox in a popover: a compact tool picker above the *same* feature
/// view the main window shows, so the app is usable with no window and no Dock
/// icon.
///
/// State lives here rather than in the `MenuBarExtra` closure so a queued set of
/// files survives the popover being dismissed — which macOS does on its own
/// whenever the panel loses focus.
struct MenuBarPanel: View {
    @ObservedObject var settings: AppSettings
    @ObservedObject var updates: UpdateController

    @State private var selection: Utility.ID = Utility.all[0].id
    @Environment(\.openWindow) private var openWindow

    private var selected: Utility {
        Utility.all.first { $0.id == selection } ?? Utility.all[0]
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            picker
            Divider()

            // Keyed the same way the main window keys its detail pane: switching
            // tools resets that tool's queue and results instead of carrying them
            // across.
            selected.makeView()
                .id(selected.id)
                .environment(\.toolPresentation, .menuBar)
        }
        .frame(width: 380)
    }

    private var header: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 1) {
                Text(selected.title)
                    .font(.subheadline.weight(.semibold))
                Text(selected.blurb)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Spacer(minLength: 8)

            menu
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    private var menu: some View {
        Menu {
            Button("Open Toolbox Window") { openMainWindow() }

            // The way back, in one click. Without a Dock icon there's no app menu
            // either, so leaving this only in Settings would make the Settings
            // window the single route out of accessory mode.
            Toggle(
                "Show Dock Icon",
                isOn: Binding(
                    get: { !settings.hidesDockIcon },
                    set: { settings.hidesDockIcon = !$0 }
                )
            )

            // Standard Settings item; works from an accessory app, where there
            // is no app menu to reach it from.
            SettingsLink {
                Text("Settings…")
            }

            if updates.isAvailable {
                Button("Check for Updates…") { updates.checkForUpdates() }
                    .disabled(!updates.canCheckForUpdates)
            }

            Divider()

            // An accessory app has no app menu, so this is the only way to quit.
            Button("Quit Toolbox") { NSApp.terminate(nil) }
                .keyboardShortcut("q")
        } label: {
            Image(systemName: "ellipsis.circle")
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel("Toolbox menu")
    }

    private var picker: some View {
        LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 6) {
            ForEach(Utility.all) { utility in
                Button {
                    selection = utility.id
                } label: {
                    HStack(spacing: 6) {
                        Text(utility.shortTitle)
                            .font(.caption)
                            .lineLimit(1)
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(
                        selection == utility.id
                            ? AnyShapeStyle(.selection)
                            : AnyShapeStyle(Color.clear),
                        in: .rect(cornerRadius: 6)
                    )
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .help(utility.blurb)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
    }

    /// From an accessory app the window opens behind whatever is frontmost, so
    /// activation has to be explicit.
    private func openMainWindow() {
        NSApp.activate(ignoringOtherApps: true)
        openWindow(id: MainWindow.id)
    }
}
