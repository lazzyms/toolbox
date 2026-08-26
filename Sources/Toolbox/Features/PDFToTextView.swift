import SwiftUI
import ToolboxKit

struct PDFToTextView: View {
    enum Style: Hashable { case plainText, markdown }

    let utility: Utility

    @State private var files: [URL] = []
    @State private var style: Style = .plainText
    @State private var separators = false
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
            runTitle: "Extract Text",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Output") {
                    Picker("", selection: $style) {
                        Text("Plain text").tag(Style.plainText)
                        Text("Markdown").tag(Style.markdown)
                    }
                    .pickerStyle(.segmented)
                }

                Toggle(isOn: $separators) {
                    VStack(alignment: .leading) {
                        Text("Page separators")
                        Text("Inserts “--- page N ---” between pages.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                OptionRow(label: "Page range") {
                    TextField("e.g. 1-3,5,7-", text: $pageRange)
                        .textFieldStyle(.roundedBorder)
                }

                DestinationPicker(location: $location)

                if style == .markdown {
                    Text("Markdown headings are guessed from font sizes — best effort, not a layout analysis. Two-column layouts extract in content-stream order.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
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
        let options = PDFTextOptions(
            style: style == .markdown ? .markdown : .plainText,
            includePageSeparators: separators
        )
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
                let output = try PDFTextExtractor.extract(url, options: options, pageRangeText: range, to: dest)
                return JobOutcome(input: url, output: output, detail: "Extracted → \(output.lastPathComponent)")
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
