import SwiftUI
import ToolboxKit

struct IconSetView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var preset: IconSetPreset = .macOS
    @State private var fill: IconFill = .crop
    @State private var customSizesText = "16, 32, 48, 64, 128, 256, 512"
    @State private var location: OutputLocation = .alongsideInput
    @State private var outcomes: [JobOutcome] = []
    @State private var isRunning = false
    @State private var progress: Double?

    @Environment(\.toolPresentation) private var presentation

    /// Deduped, sorted, and ignoring anything that isn't a positive integer.
    private var customSizes: [Int] {
        Array(Set(customSizesText
            .split(separator: ",")
            .compactMap { Int($0.trimmingCharacters(in: .whitespaces)) }
            .filter { $0 > 0 }))
            .sorted()
    }

    var body: some View {
        ToolScaffold(
            utility: utility,
            fileCount: files.count,
            isRunning: isRunning,
            progress: progress,
            runTitle: "Generate \(preset.displayName) Icons",
            canRun: !files.isEmpty && (preset != .custom || !customSizes.isEmpty),
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Preset") {
                    Picker("", selection: $preset) {
                        ForEach(IconSetPreset.allCases) { preset in
                            Text(preset.displayName).tag(preset)
                        }
                    }
                    .labelsHidden()
                    .optionPickerStyle(presentation)
                }
                Text(preset.summary)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.leading, presentation.explanationInset)
                    .fixedSize(horizontal: false, vertical: true)

                if preset == .custom {
                    OptionRow(label: "Sizes") {
                        TextField("16, 32, 48, 64", text: $customSizesText)
                            .textFieldStyle(.roundedBorder)
                            .frame(maxWidth: 220)
                            .multilineTextAlignment(.trailing)
                            .accessibilityLabel("Custom sizes in pixels, comma separated")
                    }
                }

                OptionRow(label: "Shape") {
                    Picker("", selection: $fill) {
                        ForEach(IconFill.allCases) { fill in
                            Text(fill.displayName).tag(fill)
                        }
                    }
                    .labelsHidden()
                    .optionPickerStyle(presentation)
                }

                ForEach(warnings, id: \.self) { warning in
                    Label(warning, systemImage: "exclamationmark.triangle")
                        .font(.caption)
                        .foregroundStyle(.orange)
                        .padding(.leading, presentation.explanationInset)
                        .fixedSize(horizontal: false, vertical: true)
                }

                DestinationPicker(location: $location)
            }
        } content: {
            DropZone(
                prompt: "Drop the source image to turn into icons",
                allowedExtensions: ImageFormat.readableExtensions,
                contentTypes: [],
                files: $files
            )

            if !files.isEmpty { FileList(files: $files) }
            if !outcomes.isEmpty { ResultsList(outcomes: outcomes) }
        }
    }

    /// Say which, and let the user choose: the run bar stays enabled, but a
    /// source that will be cropped or upscaled says so before it runs.
    private var warnings: [String] {
        guard let first = files.first,
              let size = IconSetGenerator.pixelSize(of: first)
        else { return [] }

        var messages: [String] = []
        if size.width != size.height {
            let verb = fill == .crop ? "center-cropped to fill" : "stretched to fill"
            messages.append(
                "Not square — \(Int(size.width))×\(Int(size.height)) will be \(verb) each icon."
            )
        }
        let shortest = min(size.width, size.height)
        if let largest = preset.largestPixels(for: customSizes),
           shortest < Double(largest) {
            messages.append(
                "Source is only \(Int(shortest))px — sizes up to \(largest)px will be upscaled."
            )
        }
        return messages
    }

    private func start() {
        guard !files.isEmpty, !isRunning else { return }
        let inputs = files
        let options = IconSetGenerator.Options(
            preset: preset,
            customSizes: preset == .custom ? customSizes : [],
            fill: fill,
            location: location
        )

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            let results = await BatchRunner.run(inputs) { done, total in
                Task { @MainActor in progress = Double(done) / Double(total) }
            } job: { url in
                let result = try IconSetGenerator.run(url, options: options)
                return JobOutcome(
                    input: url,
                    outputs: result.outputs,
                    detail: "\(result.outputs.count) files in “\(result.directory.lastPathComponent)”"
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
