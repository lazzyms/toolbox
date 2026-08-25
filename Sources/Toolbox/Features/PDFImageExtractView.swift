import SwiftUI
import ToolboxKit

struct PDFImageExtractView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var minSize = 32
    @State private var pageRange = ""
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
            runTitle: "Extract Images",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Minimum size") {
                    Picker("", selection: $minSize) {
                        Text("Any").tag(1)
                        Text("32 px").tag(32)
                        Text("64 px").tag(64)
                        Text("128 px").tag(128)
                    }
                    .pickerStyle(.segmented)
                }

                OptionRow(label: "Page range") {
                    TextField("e.g. 1-3,5,7-", text: $pageRange)
                        .textFieldStyle(.roundedBorder)
                }

                DestinationPicker(location: $location)

                Text("Embedded pictures come out at their original resolution — JPEGs stay JPEG, the rest become PNG. Skipped images are counted, never guessed at.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        } content: {
            DropZone(
                prompt: "Drop PDFs here",
                allowedExtensions: ["pdf"],
                contentTypes: [.pdf],
                files: $files
            )

            if !files.isEmpty {
                FileList(files: $files)
            }

            if !outcomes.isEmpty {
                ResultsList(outcomes: outcomes)
            }
        }
    }

    private var canRun: Bool {
        !files.isEmpty && !isRunning
    }

    private func start() {
        guard canRun else { return }
        let inputs = files
        let options = PDFExtractImagesOptions(minSize: minSize)
        let range = pageRange.isEmpty ? nil : pageRange
        let dest = location

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            let results = await BatchRunner.run(inputs) { progressCount, total in
                Task { @MainActor in
                    progress = Double(progressCount) / Double(total)
                }
            } job: { url in
                let extraction = try PDFImageExtractor.extract(url, options: options, pageRangeText: range, to: dest)
                return JobOutcome(input: url, outputs: extraction.outputs, detail: Self.detail(for: extraction))
            }

            await MainActor.run {
                outcomes = results
                isRunning = false
                progress = nil
                files = results.filter { !$0.succeeded }.map(\.input)
            }
        }
    }

    /// Runs off the main actor inside BatchRunner's job closure.
    private nonisolated static func detail(for extraction: PDFImageExtraction) -> String {
        if extraction.outputs.isEmpty && extraction.duplicates == 0
            && extraction.belowMinSize == 0 && extraction.unsupported == 0 {
            return "No embedded images"
        }
        var parts = ["\(extraction.outputs.count) images"]
        if extraction.unsupported > 0 {
            parts.append("\(extraction.unsupported) skipped (unsupported)")
        }
        if extraction.belowMinSize > 0 {
            parts.append("\(extraction.belowMinSize) below minimum size")
        }
        if extraction.duplicates > 0 {
            parts.append("\(extraction.duplicates) duplicates merged")
        }
        return parts.joined(separator: " · ")
    }
}
