import SwiftUI
import ToolboxKit

struct PDFOCRView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var dpi = 150
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
            runTitle: "Recognise Text",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Resolution") {
                    Picker("", selection: $dpi) {
                        Text("72 dpi").tag(72)
                        Text("150 dpi").tag(150)
                        Text("300 dpi").tag(300)
                    }
                    .pickerStyle(.segmented)
                }

                OptionRow(label: "Page range") {
                    TextField("e.g. 1-3,5,7-", text: $pageRange)
                        .textFieldStyle(.roundedBorder)
                }

                DestinationPicker(location: $location)

                Text("Reads text out of scans with on-device OCR — English, accurate mode. Slow on long documents; a page with nothing legible comes out empty.")
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
        let options = PDFOCROptions(dpi: dpi)
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
                let output = try PDFOCR.recognize(url, options: options, pageRangeText: range, to: dest)
                let count = (try? String(contentsOf: output, encoding: .utf8))?.count ?? 0
                return JobOutcome(
                    input: url,
                    output: output,
                    detail: "\(count) characters → \(output.lastPathComponent)"
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
