import SwiftUI
import ToolboxKit

/// The multi-page TIFF pane. One sidebar entry drives both directions through
/// a mode switch: split tears a scanner-style file into one image per page,
/// combine binds a batch of stills back into one multi-page file.
struct TIFFView: View {
    enum Mode {
        case split
        case combine
    }

    let utility: Utility

    @State private var mode: Mode = .split
    @State private var outputFormat: ImageFormat = .png
    @State private var compression: TIFFTools.Compression = .lzw
    @State private var files: [URL] = []
    @State private var location: OutputLocation = .alongsideInput
    @State private var outcomes: [JobOutcome] = []
    @State private var isRunning = false
    @State private var progress: Double?

    @Environment(\.toolPresentation) private var presentation

    var body: some View {
        ToolScaffold(
            utility: utility,
            fileCount: files.count,
            isRunning: isRunning,
            progress: progress,
            runTitle: mode == .split ? "Split Pages" : "Combine Images",
            canRun: canRun,
            run: start
        ) {
            OptionRow(label: "Mode") {
                Picker("", selection: $mode) {
                    Text("Split").tag(Mode.split)
                    Text("Combine").tag(Mode.combine)
                }
                .pickerStyle(.segmented)
                .fixedSize()
            }

            switch mode {
            case .split:
                OptionRow(label: "Output") {
                    Picker("", selection: $outputFormat) {
                        Text("PNG").tag(ImageFormat.png)
                        Text("TIFF").tag(ImageFormat.tiff)
                    }
                    .labelsHidden()
                    .frame(maxWidth: 160)
                }
                Text("Every page becomes its own “name-frame-N” file. A single-page "
                    + "TIFF splits to exactly one image.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.leading, presentation.explanationInset)
                    .fixedSize(horizontal: false, vertical: true)

            case .combine:
                OptionRow(label: "Compression") {
                    Picker("", selection: $compression) {
                        ForEach(TIFFTools.Compression.allCases, id: \.self) { choice in
                            Text(choice.displayName).tag(choice)
                        }
                    }
                    .labelsHidden()
                    .frame(maxWidth: 160)
                }
                Text("Some document-management software only accepts specific "
                    + "compressions. Pages are written in queue order — drag rows to reorder.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.leading, presentation.explanationInset)
                    .fixedSize(horizontal: false, vertical: true)
            }

            DestinationPicker(location: $location)
        } content: {
            DropZone(
                prompt: mode == .split ? "Drop multi-page TIFFs" : "Drop images to combine",
                allowedExtensions: mode == .split
                    ? ["tif", "tiff"]
                    : ImageFormat.readableExtensions,
                contentTypes: [],
                files: $files
            )

            if !files.isEmpty {
                // Order becomes page order when combining.
                FileList(files: $files, reorderable: mode == .combine)
            }
            if !outcomes.isEmpty { ResultsList(outcomes: outcomes) }
        }
        .onChange(of: mode) {
            // The two modes take different inputs; a queue built for one would
            // only confuse the other.
            files = []
            outcomes = []
        }
    }

    private var canRun: Bool {
        !files.isEmpty && !isRunning
    }

    private func start() {
        guard canRun else { return }
        let inputs = files
        let dest = location
        let format = outputFormat
        let compression = compression
        let mode = mode

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            let results: [JobOutcome]
            switch mode {
            case .split:
                results = await BatchRunner.run(inputs) { done, total in
                    Task { @MainActor in progress = Double(done) / Double(total) }
                } job: { url in
                    let outputs = try TIFFTools.split(url, format: format, to: dest)
                    return JobOutcome(
                        input: url,
                        outputs: outputs,
                        detail: "\(outputs.count) page\(outputs.count == 1 ? "" : "s")"
                    )
                }
            case .combine:
                let result = await BatchRunner.runSingle(inputs) { done, total in
                    Task { @MainActor in progress = Double(done) / Double(total) }
                } job: { urls, consume in
                    for _ in urls {
                        consume()
                    }
                    let output = try TIFFTools.combine(urls, to: dest, compression: compression)
                    return JobOutcome(
                        input: urls[0],
                        outputs: [output],
                        detail: "\(urls.count) image\(urls.count == 1 ? "" : "s") → \(output.lastPathComponent)"
                    )
                }
                results = [result]
            }

            await MainActor.run {
                outcomes = results
                isRunning = false
                progress = nil
                files = []
            }
        }
    }
}
