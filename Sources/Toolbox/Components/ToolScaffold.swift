import AppKit
import SwiftUI
import ToolboxKit

/// Standard chrome shared by every tool pane: header, scrolling body, run bar.
struct ToolScaffold<Body: View, Controls: View>: View {
    let utility: Utility
    let fileCount: Int
    let isRunning: Bool
    let progress: Double?
    let runTitle: String
    let canRun: Bool
    let run: () -> Void
    @ViewBuilder let controls: () -> Controls
    @ViewBuilder let content: () -> Body

    @Environment(\.toolPresentation) private var presentation

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: presentation.contentSpacing) {
                    // The menu bar panel puts the title and blurb in its own
                    // header next to the tool menu; repeating them here would
                    // spend a third of the popover on chrome.
                    if !presentation.isCompact {
                        HStack(spacing: 10) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(utility.title).font(.title3.weight(.semibold))
                                Text(utility.blurb)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                        }
                    }

                    content()
                    controls()
                }
                .padding(presentation.padding)
            }
            // Bounded in the panel so a long file queue can't push the popover
            // past the bottom of the screen; the window resizes instead.
            .frame(maxHeight: presentation.bodyMaxHeight)

            Divider()

            HStack(spacing: 12) {
                if isRunning {
                    if let progress {
                        ProgressView(value: progress)
                            .frame(width: 130)
                    } else {
                        ProgressView().controlSize(.small)
                    }
                    Text("Working…")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else if fileCount > 0 {
                    Text("\(fileCount) file\(fileCount == 1 ? "" : "s") ready")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                Spacer()

                Button(runTitle, action: run)
                    .keyboardShortcut(.return, modifiers: .command)
                    .buttonStyle(.borderedProminent)
                    .controlSize(presentation.runButtonSize)
                    .disabled(!canRun || isRunning)
            }
            .padding(.horizontal, presentation.padding)
            .padding(.vertical, presentation.isCompact ? 8 : 12)
            .background(.bar)
        }
    }
}

/// "Save to: alongside originals / a folder" control.
struct DestinationPicker: View {
    @Binding var location: OutputLocation

    @Environment(\.toolPresentation) private var presentation

    private var folderName: String? {
        if case .directory(let url) = location { return url.lastPathComponent }
        return nil
    }

    var body: some View {
        HStack(spacing: 8) {
            Text("Save to")
                .font(.subheadline.weight(.medium))

            Picker("", selection: Binding(
                get: { folderName == nil },
                set: { alongside in
                    if alongside { location = .alongsideInput } else { chooseFolder() }
                }
            )) {
                Text("Next to originals").tag(true)
                Text(folderName ?? "Choose folder…").tag(false)
            }
            .labelsHidden()
            .pickerStyle(.segmented)
            .fixedSize()

            if let folderName {
                Button {
                    chooseFolder()
                } label: {
                    Image(systemName: "folder")
                }
                .buttonStyle(.borderless)
                .help("Currently saving to “\(folderName)”. Click to change.")
            }

            Spacer()
        }
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"

        // A sheet on the popover, for the same reason as DropZone: a modal panel
        // would take key status away and dismiss the popover under it.
        if presentation.isCompact, let host = NSApp.keyWindow {
            panel.beginSheetModal(for: host) { response in
                if response == .OK, let url = panel.url {
                    location = .directory(url)
                } else if folderName == nil {
                    location = .alongsideInput
                }
            }
            return
        }

        NSApp.activate(ignoringOtherApps: true)
        if panel.runModal() == .OK, let url = panel.url {
            location = .directory(url)
        } else if folderName == nil {
            location = .alongsideInput
        }
    }
}

/// A labelled row used for the option controls.
struct OptionRow<Content: View>: View {
    let label: String
    @ViewBuilder let content: () -> Content

    @Environment(\.toolPresentation) private var presentation

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(label)
                .font(.subheadline.weight(.medium))
                .frame(width: presentation.optionLabelWidth, alignment: .leading)
            content()
            Spacer(minLength: 0)
        }
    }
}
