import SwiftUI
import ToolboxKit
import UniformTypeIdentifiers

struct PDFMergeView: View {
    let utility: Utility

    @State private var files: [URL] = []
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
            runTitle: "Merge PDFs",
            canRun: files.count >= 2,
            run: start
        ) {
            DestinationPicker(location: $location)
        } content: {
            DropZone(
                prompt: "Drop PDFs to merge",
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

    private func start() {
        guard files.count >= 2, !isRunning else { return }
        let inputs = files
        let dest = location

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            let result = await BatchRunner.runSingle(inputs) { progressCount, total in
                Task { @MainActor in
                    progress = Double(progressCount) / Double(total)
                }
            } job: { urls, consume in
                var consumed = 0
                for _ in urls {
                    consume()
                }
                let output = try PDFMerger.merge(urls, to: dest)
                return JobOutcome(
                    input: urls[0],
                    outputs: [output],
                    detail: "Merged \(urls.count) PDFs → \(output.lastPathComponent)"
                )
            }

            await MainActor.run {
                outcomes = [result]
                isRunning = false
                progress = nil
            }
        }
    }
}
