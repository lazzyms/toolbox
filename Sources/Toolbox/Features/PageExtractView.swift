import SwiftUI
import ToolboxKit

struct PageExtractView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var pagesText = ""
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
            runTitle: "Extract Pages",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Pages") {
                    TextField("e.g. 1-3, 7, 9-", text: $pagesText)
                        .textFieldStyle(.roundedBorder)
                }

                DestinationPicker(location: $location)

                Text(
                    "Overlapping or repeated picks include each page once, and pages come out in page order. Saved as “name-pages.pdf”."
                )
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
            && !pagesText.trimmingCharacters(in: .whitespaces).isEmpty
    }

    private func start() {
        guard canRun else { return }
        let inputs = files
        let selection = pagesText
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
                let output = try PageExtractor.extract(url, selection: selection, to: dest)
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
