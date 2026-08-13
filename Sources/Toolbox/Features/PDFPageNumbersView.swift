import SwiftUI
import ToolboxKit
import UniformTypeIdentifiers

struct PDFPageNumbersView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var position: PDFPageNumberer.Position = .bottomCenter
    @State private var startNumber = 1
    @State private var pageRange = ""
    @State private var format: PDFPageNumberer.Format = .plain
    @State private var fontSize: CGFloat = 12
    @State private var margin: CGFloat = 36
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
            runTitle: "Add Page Numbers",
            canRun: !files.isEmpty,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Position") {
                    Picker("", selection: $position) {
                        Text("Top Left").tag(PDFPageNumberer.Position.topLeft)
                        Text("Top Center").tag(PDFPageNumberer.Position.topCenter)
                        Text("Top Right").tag(PDFPageNumberer.Position.topRight)
                        Text("Bottom Left").tag(PDFPageNumberer.Position.bottomLeft)
                        Text("Bottom Center").tag(PDFPageNumberer.Position.bottomCenter)
                        Text("Bottom Right").tag(PDFPageNumberer.Position.bottomRight)
                    }
                    .pickerStyle(.segmented)
                }

                OptionRow(label: "Start number") {
                    Stepper("\(startNumber)", value: $startNumber, in: 1...9999)
                }

                OptionRow(label: "Page range") {
                    TextField("e.g. 1-3,5,7-", text: $pageRange)
                        .textFieldStyle(.roundedBorder)
                }

                OptionRow(label: "Format") {
                    Picker("", selection: $format) {
                        Text("1").tag(PDFPageNumberer.Format.plain)
                        Text("Page 1").tag(PDFPageNumberer.Format.pagePrefix)
                        Text("1 of N").tag(PDFPageNumberer.Format.pageOfTotal)
                    }
                    .pickerStyle(.segmented)
                }

                OptionRow(label: "Font size") {
                    Stepper("\(Int(fontSize)) pt", value: $fontSize, in: 6...72, step: 1)
                }

                OptionRow(label: "Margin") {
                    Stepper("\(Int(margin)) pt", value: $margin, in: 0...200, step: 4)
                }

                DestinationPicker(location: $location)
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

    private func start() {
        guard !files.isEmpty, !isRunning else { return }
        let inputs = files
        let pos = position
        let start = startNumber
        let range = pageRange.isEmpty ? nil : pageRange
        let fmt = format
        let size = fontSize
        let mar = margin
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
                let output = try PDFPageNumberer.addNumbers(
                    to: url,
                    position: pos,
                    startNumber: start,
                    pageRangeText: range,
                    format: fmt,
                    fontSize: size,
                    margin: mar,
                    to: dest
                )
                return JobOutcome(input: url, output: output, detail: "Numbered → \(output.lastPathComponent)")
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
