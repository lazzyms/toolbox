import PDFKit
import SwiftUI
import ToolboxKit

struct PDFRemovePagesView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var pagesText = ""
    @State private var location: OutputLocation = .alongsideInput
    @State private var outcomes: [JobOutcome] = []
    @State private var isRunning = false
    @State private var progress: Double?
    @State private var pageSummaries: [String] = []

    var body: some View {
        ToolScaffold(
            utility: utility,
            fileCount: files.count,
            isRunning: isRunning,
            progress: progress,
            runTitle: "Remove Pages",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Pages to remove") {
                    TextField("e.g. 2, 5-7, 11-", text: $pagesText)
                        .textFieldStyle(.roundedBorder)
                }

                DestinationPicker(location: $location)

                Text("The surviving pages keep their original order. A selection that would leave no pages at all is refused.")
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
            .onChange(of: files) { _, newValue in
                refreshPageCounts(newValue)
            }

            if !files.isEmpty {
                FileList(files: $files)

                if !pageSummaries.isEmpty {
                    Text(pageSummaries.joined(separator: " · "))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
            }

            if !outcomes.isEmpty {
                ResultsList(outcomes: outcomes)
            }
        }
    }

    // Page counts come from opening each file, which isn't work the main
    // thread should do for a large drop.
    private func refreshPageCounts(_ urls: [URL]) {
        guard !urls.isEmpty else {
            pageSummaries = []
            return
        }
        Task.detached(priority: .utility) {
            let summaries = urls.map { url -> String in
                let count = PDFDocument(url: url)?.pageCount ?? 0
                return "\(url.lastPathComponent): \(count) page\(count == 1 ? "" : "s")"
            }
            await MainActor.run {
                pageSummaries = summaries
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
        let options = PDFPageRemoveOptions(pages: pagesText)
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
                let output = try PDFPageRemover.remove(url, options: options, to: dest)
                return JobOutcome(input: url, output: output, detail: "Trimmed → \(output.lastPathComponent)")
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
