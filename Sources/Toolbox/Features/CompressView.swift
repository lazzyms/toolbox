import SwiftUI
import ToolboxKit

struct CompressView: View {
    let utility: Utility

    /// Lossless keeps every pixel; lossy re-encodes to hit a smaller size.
    private enum Mode: String, CaseIterable, Identifiable {
        case lossless, lossy
        var id: String { rawValue }
        var title: String { self == .lossless ? "Lossless" : "Lossy" }
        var explanation: String {
            switch self {
            case .lossless:
                return "Keeps each file in its own format and re-encodes with optimal settings — modest savings, no visible quality loss. If a file can't be made smaller, the original is copied unchanged."
            case .lossy:
                return "Much smaller files by discarding detail the eye is least likely to notice. The original is never modified."
            }
        }
    }

    @State private var files: [URL] = []
    @State private var mode: Mode = .lossless
    @State private var quality = 0.75
    @State private var lossyFormat: ImageFormat = .jpeg
    @State private var stripMetadata = true
    @State private var location: OutputLocation = .alongsideInput
    @State private var outcomes: [JobOutcome] = []
    @State private var isRunning = false
    @State private var progress: Double?

    @Environment(\.toolPresentation) private var presentation

    private var lossyChoices: [ImageFormat] {
        ImageFormat.encodable.filter { !$0.isLossless }
    }

    var body: some View {
        ToolScaffold(
            utility: utility,
            fileCount: files.count,
            isRunning: isRunning,
            progress: progress,
            runTitle: "Compress",
            canRun: !files.isEmpty,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Mode") {
                    Picker("", selection: $mode) {
                        ForEach(Mode.allCases) { Text($0.title).tag($0) }
                    }
                    .labelsHidden()
                    .optionPickerStyle(presentation)
                }

                Text(mode.explanation)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.leading, presentation.explanationInset)
                    .fixedSize(horizontal: false, vertical: true)

                if mode == .lossy {
                    OptionRow(label: "Format") {
                        Picker("", selection: $lossyFormat) {
                            ForEach(lossyChoices) { Text($0.displayName).tag($0) }
                        }
                        .labelsHidden()
                        .optionPickerStyle(presentation)
                    }

                    OptionRow(label: "Quality") {
                        Slider(value: $quality, in: 0.2...0.95)
                            .frame(maxWidth: 220)
                        Text("\(Int(quality * 100))%")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                            .frame(width: 38, alignment: .trailing)
                    }
                }

                OptionRow(label: "Metadata") {
                    Toggle("Remove EXIF and location data", isOn: $stripMetadata)
                }

                DestinationPicker(location: $location)
            }
        } content: {
            DropZone(
                prompt: "Drop images to make smaller",
                allowedExtensions: ImageFormat.readableExtensions,
                contentTypes: [],
                files: $files
            )

            if !files.isEmpty { FileList(files: $files) }
            if !outcomes.isEmpty { ResultsList(outcomes: outcomes) }
        }
    }

    private func start() {
        guard !files.isEmpty, !isRunning else { return }
        let inputs = files
        let options = ImageProcessor.Options(
            // Lossless keeps each file's own format — forcing PNG would balloon a
            // HEIC photo to several times its original size.
            format: mode == .lossless ? nil : lossyFormat,
            quality: mode == .lossless ? 1.0 : quality,
            stripMetadata: stripMetadata,
            // Either mode can produce a larger file than it started with, and
            // "compress" must never do that.
            keepSmallerOriginal: true,
            suffix: "-compressed",
            location: location
        )

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            let processor = ImageProcessor()
            let results = await BatchRunner.run(inputs) { done, total in
                Task { @MainActor in progress = Double(done) / Double(total) }
            } job: { url in
                let result = try processor.run(url, options: options)
                let detail = result.keptOriginal
                    ? "Already optimal — copied unchanged (\(ByteFormat.string(result.newBytes)))"
                    : ByteFormat.savings(from: result.originalBytes, to: result.newBytes)
                return JobOutcome(input: url, output: result.output, detail: detail)
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
