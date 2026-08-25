import SwiftUI
import ToolboxKit

struct PDFToImagesView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var format: ImageFormat = .jpeg
    @State private var quality = 0.85
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
            runTitle: "Export Pages",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Format") {
                    Picker("", selection: $format) {
                        Text("JPEG").tag(ImageFormat.jpeg)
                        Text("PNG").tag(ImageFormat.png)
                    }
                    .pickerStyle(.segmented)
                }

                if format == .jpeg {
                    OptionRow(label: "Quality") {
                        Slider(value: $quality, in: 0.1...1.0) {
                            Text("\(Int(quality * 100))%")
                        } minimumValueLabel: { Text("10%") } maximumValueLabel: { Text("100%") }
                    }
                }

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

                Text("Pages become pixels — text is no longer selectable in the output.")
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
        !files.isEmpty && !isRunning && (format != .jpeg || quality > 0)
    }

    private func start() {
        guard canRun else { return }
        let inputs = files
        let options = PDFToImagesOptions(
            format: format,
            quality: format == .jpeg ? quality : 0.85,
            dpi: dpi
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
                let outputs = try PDFImageExporter.convert(url, options: options, pageRangeText: range, to: dest)
                return JobOutcome(
                    input: url,
                    outputs: outputs,
                    detail: "\(outputs.count) pages → \(options.format.displayName)"
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
