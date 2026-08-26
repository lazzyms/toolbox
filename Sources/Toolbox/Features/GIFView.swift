import SwiftUI
import ToolboxKit

/// Which direction of `GIFBuilder` this pane drives. Fixed per tool in the
/// registry, so each of "Create GIF" and "Extract GIF Frames" gets its own
/// sidebar entry without any mode switching inside the pane.
enum GIFMode {
    case create
    case extract
}

struct GIFView: View {
    let utility: Utility
    let mode: GIFMode

    @State private var files: [URL] = []
    @State private var frameDelay = 0.1
    @State private var loopCount = 0
    @State private var maxDimension = 1280
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
            runTitle: mode == .create ? "Create GIF" : "Extract Frames",
            canRun: !files.isEmpty,
            run: start
        ) {
            switch mode {
            case .create:
                OptionRow(label: "Frame delay") {
                    Slider(value: $frameDelay, in: 0.05...1.0, step: 0.05)
                        .frame(maxWidth: 200)
                    Text(delayText)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(width: 42, alignment: .trailing)
                }

                OptionRow(label: "Loop") {
                    Picker("", selection: $loopCount) {
                        Text("Forever").tag(0)
                        Text("Once").tag(1)
                        Text("2 times").tag(2)
                        Text("3 times").tag(3)
                        Text("5 times").tag(5)
                        Text("10 times").tag(10)
                    }
                    .labelsHidden()
                    .frame(maxWidth: 160)
                }
                Text("Drop the frames in the order you want them to play, then drag rows to reorder.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.leading, presentation.explanationInset)
                    .fixedSize(horizontal: false, vertical: true)

                OptionRow(label: "Max size") {
                    Picker("", selection: $maxDimension) {
                        Text("No limit").tag(0)
                        Text("480 px").tag(480)
                        Text("640 px").tag(640)
                        Text("800 px").tag(800)
                        Text("1280 px").tag(1280)
                        Text("1920 px").tag(1920)
                    }
                    .labelsHidden()
                    .frame(maxWidth: 160)
                }
                Text("Caps the longest side of every frame, so a drop of full-resolution photos can't become a multi-hundred-MB GIF.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.leading, presentation.explanationInset)
                    .fixedSize(horizontal: false, vertical: true)

            case .extract:
                Text("Every frame becomes its own PNG alongside the original.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.leading, presentation.explanationInset)
                    .fixedSize(horizontal: false, vertical: true)
            }

            DestinationPicker(location: $location)
        } content: {
            DropZone(
                prompt: mode == .create ? "Drop images to animate" : "Drop animated GIFs",
                allowedExtensions: mode == .create
                    ? ImageFormat.readableExtensions
                    : ["gif"],
                contentTypes: [],
                files: $files
            )

            if !files.isEmpty {
                // Order is frame order when creating, so allow reordering.
                FileList(files: $files, reorderable: mode == .create)
            }
            if !outcomes.isEmpty { ResultsList(outcomes: outcomes) }
        }
    }

    private var delayText: String { String(format: "%g s", frameDelay) }

    private func start() {
        guard !files.isEmpty, !isRunning else { return }
        let inputs = files
        let mode = mode
        let location = location
        let options = GIFBuilder.CreateOptions(
            frameDelay: frameDelay,
            loopCount: loopCount,
            maxDimension: maxDimension,
            location: location
        )
        let extractOptions = GIFBuilder.ExtractOptions(location: location)

        isRunning = true
        progress = mode == .create ? nil : 0
        outcomes = []

        Task {
            let results: [JobOutcome]
            switch mode {
            case .create:
                results = await Task.detached {
                    Self.createJob(from: inputs, options: options)
                }.value
            case .extract:
                results = await BatchRunner.run(inputs) { done, total in
                    Task { @MainActor in progress = Double(done) / Double(total) }
                } job: { url in
                    let extraction = try GIFBuilder.extractFrames(from: url, options: extractOptions)
                    return JobOutcome(
                        input: url,
                        outputs: extraction.outputs,
                        detail: "\(extraction.outputs.count) frames · \(String(format: "%.1f", extraction.totalDuration)) s"
                    )
                }
            }

            await MainActor.run {
                outcomes = results
                isRunning = false
                progress = nil
                files = []
            }
        }
    }

    /// Create is many files → one GIF, so it can't go through `BatchRunner`'s
    /// per-file model; a single job reports the whole animation. Runs detached
    /// because decoding and encoding a batch is CPU-heavy and must not block the
    /// main actor. `GIFBuilder.createGIF` itself guards against an empty input.
    nonisolated static func createJob(from inputs: [URL], options: GIFBuilder.CreateOptions) -> [JobOutcome] {
        // `start` never runs with an empty queue, but `createGIF` itself guards
        // the empty case too — and this must not index `inputs[0]` if it ever
        // happens to arrive empty.
        guard let first = inputs.first else { return [] }
        do {
            let result = try GIFBuilder.createGIF(from: inputs, options: options)
            let size = ByteFormat.string(OutputNaming.fileSize(of: result.output))
            return [JobOutcome(
                input: first,
                output: result.output,
                detail: "\(result.frameCount) frames · \(String(format: "%.1f", result.framesPerSecond)) fps · \(size)"
            )]
        } catch {
            return [JobOutcome(
                input: first,
                output: nil,
                detail: "",
                failure: (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
            )]
        }
    }
}
