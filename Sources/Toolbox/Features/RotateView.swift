import SwiftUI
import ToolboxKit

struct RotateView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var degrees = 90
    @State private var flipHorizontal = false
    @State private var flipVertical = false
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
            runTitle: "Rotate",
            canRun: !files.isEmpty && spec.isActive,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Rotation") {
                    Picker("Rotation", selection: $degrees) {
                        ForEach([0, 90, 180, 270], id: \.self) { degrees in
                            Text("\(degrees)°").tag(degrees)
                        }
                    }
                    .labelsHidden()
                    .optionPickerStyle(presentation)
                }
                Text("Quarter turns relabel pixels instead of resampling, so a 90° rotation costs no quality — unlike an arbitrary angle.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.leading, presentation.explanationInset)
                    .fixedSize(horizontal: false, vertical: true)

                OptionRow(label: "Flip") {
                    Toggle("Horizontal", isOn: $flipHorizontal)
                    Toggle("Vertical", isOn: $flipVertical)
                }

                DestinationPicker(location: $location)
            }
        } content: {
            DropZone(
                prompt: "Drop images to rotate or flip",
                allowedExtensions: ImageFormat.readableExtensions,
                contentTypes: [],
                files: $files
            )

            if !files.isEmpty { FileList(files: $files) }
            if !outcomes.isEmpty { ResultsList(outcomes: outcomes) }
        }
    }

    private var spec: RotateSpec {
        RotateSpec(degrees: degrees, flipHorizontal: flipHorizontal, flipVertical: flipVertical)
    }

    /// "90° + flip H", the verb that describes what this run did to each file.
    private var action: String {
        var parts: [String] = []
        if degrees != 0 { parts.append("\(degrees)°") }
        if flipHorizontal { parts.append("flip H") }
        if flipVertical { parts.append("flip V") }
        return parts.joined(separator: " + ")
    }

    private func start() {
        guard !files.isEmpty, spec.isActive, !isRunning else { return }
        let inputs = files
        let action = action
        let options = ImageProcessor.Options(
            // nil format keeps each file in its original format, so a mixed
            // batch of PNGs and JPEGs stays that way.
            format: nil,
            quality: 0.9,
            operations: spec.operations,
            suffix: "-rotated",
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
                let from = "\(Int(result.originalPixelSize.width))×\(Int(result.originalPixelSize.height))"
                let to = "\(Int(result.pixelSize.width))×\(Int(result.pixelSize.height))"
                return JobOutcome(
                    input: url,
                    output: result.output,
                    detail: "\(action) · \(from) → \(to) px · \(ByteFormat.savings(from: result.originalBytes, to: result.newBytes))"
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
