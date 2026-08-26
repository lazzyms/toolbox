import SwiftUI
import ToolboxKit

struct PDFSplitView: View {
    enum Mode: Hashable { case everyPage, ranges, everyN }

    let utility: Utility

    @State private var files: [URL] = []
    @State private var mode: Mode = .everyPage
    @State private var rangesText = ""
    @State private var chunkSize = 2.0
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
            runTitle: "Split PDF",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Mode") {
                    Picker("", selection: $mode) {
                        Text("Every page").tag(Mode.everyPage)
                        Text("Ranges").tag(Mode.ranges)
                        Text("Every N").tag(Mode.everyN)
                    }
                    .pickerStyle(.segmented)
                }

                if mode == .ranges {
                    OptionRow(label: "Ranges") {
                        TextField("e.g. 1-3, 4-8, 9-", text: $rangesText)
                            .textFieldStyle(.roundedBorder)
                    }
                } else if mode == .everyN {
                    OptionRow(label: "Pages per file") {
                        Stepper("\(Int(chunkSize))", value: $chunkSize, in: 1...100, step: 1)
                    }
                }

                DestinationPicker(location: $location)

                Text(mode == .ranges
                     ? "Each range becomes its own file; pages outside every range are dropped."
                     : "A large document can write hundreds of files — pick a folder in “Save to” first.")
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
        guard !files.isEmpty, !isRunning else { return false }
        switch mode {
        case .everyPage:
            return true
        case .ranges:
            return !rangesText.trimmingCharacters(in: .whitespaces).isEmpty
        case .everyN:
            return Int(chunkSize) >= 1
        }
    }

    private func start() {
        guard canRun else { return }
        let inputs = files
        let splitMode: PDFSplitOptions.Mode
        switch mode {
        case .everyPage:
            splitMode = .everyPage
        case .ranges:
            splitMode = .ranges(rangesText)
        case .everyN:
            splitMode = .everyN(Int(chunkSize))
        }
        let options = PDFSplitOptions(mode: splitMode)
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
                let outputs = try PDFSplitter.split(url, options: options, to: dest)
                return JobOutcome(
                    input: url,
                    outputs: outputs,
                    detail: "Split into \(outputs.count) files"
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
