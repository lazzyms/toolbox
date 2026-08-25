import SwiftUI
import ToolboxKit

struct ImageMetadataView: View {
    enum Mode: Hashable { case view, strip }

    let utility: Utility

    @State private var files: [URL] = []
    @State private var mode: Mode = .view
    @State private var selected: URL?
    @State private var rows: [(group: String, key: String, value: String)] = []
    @State private var loadError: String?
    @State private var stripMode: ImageMetadata.StripMode = .everything
    @State private var location: OutputLocation = .alongsideInput
    @State private var outcomes: [JobOutcome] = []
    @State private var isRunning = false
    @State private var progress: Double?

    var body: some View {
        ToolScaffold(
            utility: utility,
            fileCount: files.count,
            isRunning: isRunning,
            progress: progress,
            runTitle: "Strip Metadata",
            canRun: mode == .strip && !files.isEmpty,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Mode") {
                    Picker("", selection: $mode) {
                        Text("View").tag(Mode.view)
                        Text("Strip").tag(Mode.strip)
                    }
                    .pickerStyle(.segmented)
                }

                switch mode {
                case .view:
                    if files.count > 1 {
                        OptionRow(label: "File") {
                            Picker("", selection: $selected) {
                                ForEach(files, id: \.self) { url in
                                    Text(url.lastPathComponent).tag(url as URL?)
                                }
                            }
                            .labelsHidden()
                        }
                    }
                    metadataSection

                case .strip:
                    OptionRow(label: "Remove") {
                        Picker("", selection: $stripMode) {
                            ForEach(ImageMetadata.StripMode.allCases) { option in
                                Text(option.label).tag(option)
                            }
                        }
                        .labelsHidden()
                        .fixedSize()
                    }
                    DestinationPicker(location: $location)

                    if stripMode == .locationOnly {
                        Text("Camera settings stay; coordinates and every other location tag go.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        } content: {
            DropZone(
                prompt: "Drop images here",
                allowedExtensions: ImageFormat.readableExtensions,
                contentTypes: [],
                files: $files
            )

            if !files.isEmpty, mode == .strip {
                FileList(files: $files)
            }

            if !outcomes.isEmpty {
                ResultsList(outcomes: outcomes)
            }
        }
        .onChange(of: files) { _, updated in
            outcomes = []
            // A dropped or removed queue can leave the selection pointing at a
            // file the viewer no longer knows about.
            guard let selected, updated.contains(selected) else {
                self.selected = updated.first
                return
            }
        }
        .task(id: selected) {
            await loadSummary()
        }
    }

    // MARK: - View mode

    @ViewBuilder
    private var metadataSection: some View {
        if let loadError {
            Text(loadError)
                .font(.caption)
                .foregroundStyle(.red)
        } else if rows.isEmpty {
            Text(selected == nil ? "Drop an image to see its metadata."
                                 : "Reading…")
                .font(.caption)
                .foregroundStyle(.secondary)
        } else {
            if hasLocation {
                Label("This photo includes location data — anyone you send it to can read where it was taken.",
                      systemImage: "location.fill")
                    .font(.callout.weight(.medium))
                    .foregroundStyle(.orange)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
            }

            let grouped = Dictionary(grouping: rows, by: \.group)
            ForEach(grouped.keys.sorted(), id: \.self) { group in
                VStack(alignment: .leading, spacing: 4) {
                    Text(group)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    ForEach(grouped[group]!, id: \.key) { row in
                        HStack(alignment: .firstTextBaseline) {
                            Text(row.key)
                                .font(.caption.monospacedDigit())
                            Spacer(minLength: 12)
                            Text(row.value.isEmpty ? "—" : row.value)
                                .font(.caption.monospacedDigit())
                                .foregroundStyle(.secondary)
                                .multilineTextAlignment(.trailing)
                        }
                    }
                }
                .padding(.vertical, 2)
            }
        }
    }

    private var hasLocation: Bool {
        rows.contains { $0.group == "GPS" }
    }

    private func loadSummary() async {
        guard let url = selected else {
            rows = []
            loadError = nil
            return
        }
        do {
            let summary = try ImageMetadata.summary(of: url)
            await MainActor.run {
                rows = summary
                loadError = nil
            }
        } catch {
            await MainActor.run {
                rows = []
                loadError = (error as? LocalizedError)?.errorDescription
                    ?? error.localizedDescription
            }
        }
    }

    // MARK: - Strip mode

    private func start() {
        guard mode == .strip, !files.isEmpty, !isRunning else { return }
        let inputs = files
        let target = stripMode
        let dest = location

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            let results = await BatchRunner.run(inputs) { done, total in
                Task { @MainActor in progress = Double(done) / Double(total) }
            } job: { url in
                let output = try ImageMetadata.strip(from: url, to: dest, mode: target)
                return JobOutcome(
                    input: url,
                    output: output,
                    detail: "Stripped → \(output.lastPathComponent)"
                )
            }

            await MainActor.run {
                outcomes = results
                isRunning = false
                progress = nil
                files = results.filter { !$0.succeeded }.map(\.input)
            }
        }
    }
}
