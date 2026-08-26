import SwiftUI
import ToolboxKit
import UniformTypeIdentifiers

struct ImagesToPDFView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var pageSize: ImagesToPDFOptions.PageSize = .fitToImage
    @State private var orientation: ImagesToPDFOptions.Orientation = .portrait
    @State private var margin = 36.0
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
            runTitle: "Create PDF",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Page size") {
                    Picker("", selection: $pageSize) {
                        Text("Fit to image").tag(ImagesToPDFOptions.PageSize.fitToImage)
                        Text("A4").tag(ImagesToPDFOptions.PageSize.a4)
                        Text("US Letter").tag(ImagesToPDFOptions.PageSize.usLetter)
                    }
                    .pickerStyle(.segmented)
                }

                if pageSize != .fitToImage {
                    OptionRow(label: "Orientation") {
                        Picker("", selection: $orientation) {
                            Text("Portrait").tag(ImagesToPDFOptions.Orientation.portrait)
                            Text("Landscape").tag(ImagesToPDFOptions.Orientation.landscape)
                        }
                        .pickerStyle(.segmented)
                    }

                    OptionRow(label: "Margin") {
                        Stepper("\(Int(margin)) pt", value: $margin, in: 0...200, step: 4)
                    }
                }

                DestinationPicker(location: $location)

                if files.count > 1 {
                    Text("Pages are written in queue order — drag to reorder.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        } content: {
            DropZone(
                prompt: "Drop images to combine",
                allowedExtensions: ["png", "jpg", "jpeg", "heic", "tif", "tiff", "webp", "gif"],
                contentTypes: [.image],
                files: $files
            )

            if !files.isEmpty {
                FileList(files: $files, reorderable: true)
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
        let options = ImagesToPDFOptions(
            pageSize: pageSize,
            orientation: orientation,
            margin: margin
        )
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
                for _ in urls {
                    consume()
                }
                let output = try ImagesToPDF.build(urls, options: options, to: dest)
                return JobOutcome(
                    input: urls[0],
                    outputs: [output],
                    detail: "\(urls.count) images → \(output.lastPathComponent)"
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
