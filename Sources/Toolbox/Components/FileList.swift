import AppKit
import SwiftUI
import ToolboxKit

/// The queue of selected files, with per-row removal.
struct FileList: View {
    @Binding var files: [URL]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("\(files.count) file\(files.count == 1 ? "" : "s")")
                    .font(.subheadline.weight(.medium))
                Spacer()
                Button("Clear All") { files.removeAll() }
                    .buttonStyle(.link)
            }

            ScrollView {
                LazyVStack(spacing: 2) {
                    ForEach(files, id: \.self) { url in
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
            }
            .frame(maxHeight: 150)
        }
    }
}

/// Per-file outcomes after a run.
struct ResultsList: View {
    let outcomes: [JobOutcome]

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
                            successes.compactMap(\.output)
                        )
                    }
                    .buttonStyle(.link)
                    .help(first.deletingLastPathComponent().path)
                }
            }

            ScrollView {
                LazyVStack(spacing: 2) {
                    ForEach(outcomes) { outcome in
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Image(systemName: outcome.succeeded
                                  ? "checkmark.circle" : "xmark.octagon.fill")
                                .foregroundStyle(outcome.succeeded ? Color.secondary : Color.red)
                                .font(.caption)

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

                            Spacer(minLength: 4)

                            if let output = outcome.output {
                                Button {
                                    NSWorkspace.shared.activateFileViewerSelecting([output])
                                } label: {
                                    Image(systemName: "folder")
                                }
                                .buttonStyle(.plain)
                                .foregroundStyle(.secondary)
                                .accessibilityLabel("Reveal \(output.lastPathComponent)")
                            }
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(Color(nsColor: .textBackgroundColor).opacity(0.5), in: .rect(cornerRadius: 6))
                    }
                }
            }
            .frame(maxHeight: 180)
        }
    }

    private var summary: String {
        if failures.isEmpty { return "Done — \(successes.count) file\(successes.count == 1 ? "" : "s")" }
        if successes.isEmpty { return "Failed — \(failures.count) file\(failures.count == 1 ? "" : "s")" }
        return "\(successes.count) done, \(failures.count) failed"
    }
}
