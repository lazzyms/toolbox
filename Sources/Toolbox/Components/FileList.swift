import AppKit
import SwiftUI
import ToolboxKit

/// The queue of selected files, with per-row removal.
struct FileList: View {
    @Binding var files: [URL]
    /// Whether the queue can be reordered by dragging. Only tools where order
    /// means something (merge) turn this on; for everything else the rows are
    /// a plain list so a stray drag can't silently reorder a batch.
    var reorderable = false

    @Environment(\.toolPresentation) private var presentation

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("\(files.count) file\(files.count == 1 ? "" : "s")")
                    .font(.subheadline.weight(.medium))
                Spacer()
                Button("Clear All") { files.removeAll() }
                    .buttonStyle(.link)
            }

            if reorderable {
                List {
                    ForEach(files, id: \.self) { url in
                        fileRow(for: url)
                            .listRowInsets(EdgeInsets(top: 1, leading: 0, bottom: 1, trailing: 0))
                            .listRowSeparator(.hidden)
                    }
                    .onMove { sources, destination in
                        files.move(fromOffsets: sources, toOffset: destination)
                    }
                }
                .listStyle(.plain)
                .scrollContentBackground(.hidden)
                .frame(maxHeight: presentation.fileListMaxHeight)
            } else {
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(files, id: \.self) { url in
                            fileRow(for: url)
                        }
                    }
                }
                .frame(maxHeight: presentation.fileListMaxHeight)
            }
        }
    }

    private func fileRow(for url: URL) -> some View {
        HStack(spacing: 8) {
            Image(nsImage: NSWorkspace.shared.icon(forFile: url.path))
                .resizable()
                .frame(width: 16, height: 16)

            Text(url.lastPathComponent)
                .lineLimit(1)
                .truncationMode(.middle)

            Spacer(minLength: 8)

            Text(ByteFormat.string(OutputNaming.fileSize(of: url)))
                .font(.caption)
                .foregroundStyle(.secondary)
                .monospacedDigit()

            Button {
                files.removeAll { $0 == url }
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.tertiary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove \(url.lastPathComponent)")
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Color(nsColor: .textBackgroundColor).opacity(0.5), in: .rect(cornerRadius: 6))
    }
}

/// Per-file outcomes after a run.
struct ResultsList: View {
    let outcomes: [JobOutcome]

    @Environment(\.toolPresentation) private var presentation
    @State private var expanded = Set<URL>()

    private var failures: [JobOutcome] { outcomes.filter { !$0.succeeded } }
    private var successes: [JobOutcome] { outcomes.filter(\.succeeded) }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: failures.isEmpty
                      ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                    .foregroundStyle(failures.isEmpty ? .green : .orange)

                Text(summary)
                    .font(.subheadline.weight(.medium))

                Spacer()

                if let first = successes.first?.output {
                    Button("Show in Finder") {
                        NSWorkspace.shared.activateFileViewerSelecting(
                            successes.flatMap(\.outputs)
                        )
                    }
                    .buttonStyle(.link)
                    .help(first.deletingLastPathComponent().path)
                }
            }

            ScrollView {
                LazyVStack(spacing: 2) {
                    ForEach(outcomes) { outcome in
                        if outcome.outputs.count > 1 {
                            multiOutputRow(outcome)
                        } else {
                            singleOutputRow(outcome)
                        }
                    }
                }
            }
            .frame(maxHeight: presentation.resultsMaxHeight)
        }
    }

    private func singleOutputRow(_ outcome: JobOutcome) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            statusIcon(outcome)
            outcomeTexts(outcome)

            Spacer(minLength: 4)

            if let output = outcome.outputs.first {
                revealButton(for: output)
            }
        }
        .rowBackground()
    }

    /// An outcome that wrote several files expands to one row per file, so a
    /// 200-page split shows a count and a chevron instead of flooding the pane.
    private func multiOutputRow(_ outcome: JobOutcome) -> some View {
        let isExpanded = expanded.contains(outcome.id)

        return VStack(alignment: .leading, spacing: 2) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                statusIcon(outcome)
                outcomeTexts(outcome)

                Spacer(minLength: 4)

                Button {
                    toggleExpanded(outcome.id)
                } label: {
                    HStack(spacing: 4) {
                        Text("\(outcome.outputs.count) files")
                            .font(.caption)
                            .monospacedDigit()
                        Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                            .font(.caption2)
                    }
                }
                .buttonStyle(.link)
                .foregroundStyle(.secondary)
                .accessibilityLabel(
                    "\(isExpanded ? "Hide" : "Show") \(outcome.outputs.count) outputs"
                )
            }

            if isExpanded {
                ForEach(outcome.outputs, id: \.self) { output in
                    HStack(spacing: 8) {
                        Text(output.lastPathComponent)
                            .font(.caption2)
                            .lineLimit(1)
                            .truncationMode(.middle)

                        Spacer()

                        revealButton(for: output)
                    }
                    .padding(.leading, 18)
                }
            }
        }
        .rowBackground()
    }

    private func statusIcon(_ outcome: JobOutcome) -> some View {
        Image(systemName: outcome.succeeded
              ? "checkmark.circle" : "xmark.octagon.fill")
            .foregroundStyle(outcome.succeeded ? Color.secondary : Color.red)
            .font(.caption)
    }

    private func outcomeTexts(_ outcome: JobOutcome) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(outcome.input.lastPathComponent)
                .font(.caption)
                .lineLimit(1)
                .truncationMode(.middle)

            Text(outcome.failure ?? outcome.detail)
                .font(.caption2)
                .foregroundStyle(outcome.succeeded ? Color.secondary : Color.red)
                .lineLimit(2)
        }
    }

    private func revealButton(for output: URL) -> some View {
        Button {
            NSWorkspace.shared.activateFileViewerSelecting([output])
        } label: {
            Image(systemName: "folder")
        }
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
        .accessibilityLabel("Reveal \(output.lastPathComponent)")
    }

    private func toggleExpanded(_ id: URL) {
        if expanded.contains(id) {
            expanded.remove(id)
        } else {
            expanded.insert(id)
        }
    }

    private var summary: String {
        if failures.isEmpty { return "Done — \(successes.count) file\(successes.count == 1 ? "" : "s")" }
        if successes.isEmpty { return "Failed — \(failures.count) file\(failures.count == 1 ? "" : "s")" }
        return "\(successes.count) done, \(failures.count) failed"
    }
}

private extension View {
    func rowBackground() -> some View {
        self.padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(Color(nsColor: .textBackgroundColor).opacity(0.5), in: .rect(cornerRadius: 6))
    }
}
