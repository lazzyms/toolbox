import SwiftUI
import ToolboxKit

struct PDFCompressView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var quality = 0.75
    @State private var dpi = 150
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
            runTitle: "Compress",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Quality") {
                    Slider(value: $quality, in: 0.1...1.0) {
                        Text("\(Int(quality * 100))%")
                    } minimumValueLabel: { Text("10%") } maximumValueLabel: { Text("100%") }
                }

                OptionRow(label: "Resolution") {
                    Picker("", selection: $dpi) {
                        Text("72 dpi").tag(72)
                        Text("150 dpi").tag(150)
                    }
                    .pickerStyle(.segmented)
                }

                DestinationPicker(location: $location)

                Text(
                    "Each page becomes a JPEG image at this resolution — text turns into pixels "
                    + "and is no longer selectable or searchable. The copy is only written when "
                    + "it's actually smaller than the original; otherwise nothing happens."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            }
        } content: {
            DropZone(
                prompt: "Drop PDFs to make smaller",
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
        !files.isEmpty && !isRunning && quality > 0
    }

    private func start() {
        guard canRun else { return }
        let inputs = files
        let options = PDFCompressOptions(dpi: dpi, quality: quality)
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
                let result = try PDFCompressor.compress(url, options: options, to: dest)
                return JobOutcome(
                    input: url,
                    output: result.output,
                    detail: ByteFormat.savings(from: result.originalBytes, to: result.newBytes)
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
