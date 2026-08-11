import SwiftUI
import ToolboxKit

struct ConvertView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var format: ImageFormat = .png
    @State private var quality = 0.9
    @State private var stripMetadata = false
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
            runTitle: "Convert to \(format.displayName)",
            canRun: !files.isEmpty,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Format") {
                    Picker("", selection: $format) {
                        // Only formats this Mac can encode, so a run can't fail
                        // at the last step.
                        ForEach(ImageFormat.encodable) { option in
                            Text(option.displayName).tag(option)
                        }
                    }
                    .labelsHidden()
                    .pickerStyle(.segmented)
                    .fixedSize()
                }

                if format.supportsQuality {
                    OptionRow(label: "Quality") {
                        Slider(value: $quality, in: 0.3...1.0)
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
                prompt: "Drop HEIC, PNG, JPEG, TIFF or RAW images here",
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
            format: format,
            quality: quality,
            stripMetadata: stripMetadata,
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
                return JobOutcome(
                    input: url,
                    output: result.output,
                    detail: "\(result.output.lastPathComponent) · \(ByteFormat.savings(from: result.originalBytes, to: result.newBytes))"
                )
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
