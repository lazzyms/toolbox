import SwiftUI
import ToolboxKit

struct PDFCropView: View {
    enum Mode: Hashable { case margins, region }

    let utility: Utility

    @State private var files: [URL] = []
    @State private var mode: Mode = .margins
    @State private var top = 0.0
    @State private var bottom = 0.0
    @State private var leading = 0.0
    @State private var trailing = 0.0
    @State private var regionX = 0.0
    @State private var regionY = 0.0
    @State private var regionWidth = 300.0
    @State private var regionHeight = 300.0
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
            runTitle: "Crop PDFs",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Mode") {
                    Picker("", selection: $mode) {
                        Text("Margins").tag(Mode.margins)
                        Text("Region").tag(Mode.region)
                    }
                    .pickerStyle(.segmented)
                }

                if mode == .margins {
                    OptionRow(label: "Top") { Stepper("\(Int(top)) pt", value: $top, in: 0...500, step: 2) }
                    OptionRow(label: "Bottom") { Stepper("\(Int(bottom)) pt", value: $bottom, in: 0...500, step: 2) }
                    OptionRow(label: "Left") { Stepper("\(Int(leading)) pt", value: $leading, in: 0...500, step: 2) }
                    OptionRow(label: "Right") { Stepper("\(Int(trailing)) pt", value: $trailing, in: 0...500, step: 2) }
                } else {
                    OptionRow(label: "X") { Stepper("\(Int(regionX)) pt", value: $regionX, in: 0...2000, step: 5) }
                    OptionRow(label: "Y") { Stepper("\(Int(regionY)) pt", value: $regionY, in: 0...2000, step: 5) }
                    OptionRow(label: "Width") { Stepper("\(Int(regionWidth)) pt", value: $regionWidth, in: 1...2000, step: 5) }
                    OptionRow(label: "Height") { Stepper("\(Int(regionHeight)) pt", value: $regionHeight, in: 1...2000, step: 5) }
                    Text("The region starts at (X, Y) from the page's bottom-left corner.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                OptionRow(label: "Page range") {
                    TextField("e.g. 1-3,5,7-", text: $pageRange)
                        .textFieldStyle(.roundedBorder)
                }

                DestinationPicker(location: $location)

                Text("Cropping hides content outside the box — it isn't removed from the file. Don't use it to hide sensitive content.")
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
        case .margins:
            return top + bottom + leading + trailing > 0
        case .region:
            return regionWidth > 0 && regionHeight > 0
        }
    }

    private func start() {
        guard canRun else { return }
        let inputs = files
        let options: PDFCropOptions
        switch mode {
        case .margins:
            options = PDFCropOptions(mode: .margins(
                top: top, bottom: bottom, left: leading, right: trailing
            ))
        case .region:
            options = PDFCropOptions(mode: .region(
                x: regionX, y: regionY, width: regionWidth, height: regionHeight
            ))
        }
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
                let output = try PDFCropper.apply(options, to: url, pageRangeText: range, to: dest)
                return JobOutcome(input: url, output: output, detail: "Cropped → \(output.lastPathComponent)")
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
