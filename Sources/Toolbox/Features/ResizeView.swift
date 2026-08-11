import SwiftUI
import ToolboxKit

struct ResizeView: View {
    let utility: Utility

    private enum Method: String, CaseIterable, Identifiable {
        case fit, longest, percent, exact
        var id: String { rawValue }
        var title: String {
            switch self {
            case .fit: return "Fit box"
            case .longest: return "Longest side"
            case .percent: return "Percentage"
            case .exact: return "Exact"
            }
        }
    }

    @State private var files: [URL] = []
    @State private var method: Method = .fit
    @State private var width = "1920"
    @State private var height = "1080"
    @State private var longest = "2048"
    @State private var percent = 50.0
    @State private var allowUpscale = false
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
            runTitle: "Resize",
            canRun: !files.isEmpty && spec.isActive,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Method") {
                    Picker("", selection: $method) {
                        ForEach(Method.allCases) { Text($0.title).tag($0) }
                    }
                    .labelsHidden()
                    .pickerStyle(.segmented)
                    .fixedSize()
                }

                switch method {
                case .fit:
                    OptionRow(label: "Max size") {
                        pixelField($width, label: "Width")
                        Text("×").foregroundStyle(.secondary)
                        pixelField($height, label: "Height")
                        Text("px").font(.caption).foregroundStyle(.secondary)
                    }
                    Text("Scales down to fit inside the box. Aspect ratio is preserved — leave a field empty to leave that side unconstrained.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.leading, 100)
                        .fixedSize(horizontal: false, vertical: true)

                case .longest:
                    OptionRow(label: "Longest side") {
                        pixelField($longest, label: "Longest side")
                        Text("px").font(.caption).foregroundStyle(.secondary)
                    }
                    Text("Handles portrait and landscape in one batch — whichever side is longer becomes this size.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.leading, 100)
                        .fixedSize(horizontal: false, vertical: true)

                case .percent:
                    OptionRow(label: "Scale") {
                        Slider(value: $percent, in: 5...200, step: 5)
                            .frame(maxWidth: 220)
                        Text("\(Int(percent))%")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                            .frame(width: 42, alignment: .trailing)
                    }

                case .exact:
                    OptionRow(label: "Size") {
                        pixelField($width, label: "Width")
                        Text("×").foregroundStyle(.secondary)
                        pixelField($height, label: "Height")
                        Text("px").font(.caption).foregroundStyle(.secondary)
                    }
                    Label("Stretches the image if this doesn't match its aspect ratio.",
                          systemImage: "exclamationmark.triangle")
                        .font(.caption)
                        .foregroundStyle(.orange)
                        .padding(.leading, 100)
                }

                if method != .exact {
                    OptionRow(label: "Upscaling") {
                        Toggle("Allow enlarging images smaller than the target", isOn: $allowUpscale)
                    }
                }

                DestinationPicker(location: $location)
            }
        } content: {
            DropZone(
                prompt: "Drop images to resize",
                allowedExtensions: ImageFormat.readableExtensions,
                contentTypes: [],
                files: $files
            )

            if !files.isEmpty { FileList(files: $files) }
            if !outcomes.isEmpty { ResultsList(outcomes: outcomes) }
        }
    }

    private func pixelField(_ binding: Binding<String>, label: String) -> some View {
        TextField(label, text: binding)
            .textFieldStyle(.roundedBorder)
            .frame(width: 72)
            .multilineTextAlignment(.trailing)
            .accessibilityLabel(label)
            .onSubmit(start)
    }

    private var spec: ResizeSpec {
        // Empty means "unconstrained" for fit; Int(...) returning nil handles
        // both empty and non-numeric input.
        switch method {
        case .fit:
            return .fit(width: Int(width), height: Int(height))
        case .longest:
            return .longestSide(Int(longest) ?? 0)
        case .percent:
            return .percent(percent)
        case .exact:
            return .exact(width: Int(width) ?? 0, height: Int(height) ?? 0)
        }
    }

    private func start() {
        guard !files.isEmpty, spec.isActive, !isRunning else { return }
        let inputs = files
        let options = ImageProcessor.Options(
            // nil format keeps each file in its original format, so a mixed
            // batch of PNGs and JPEGs stays that way.
            format: nil,
            quality: 0.9,
            resize: spec,
            allowUpscale: method == .exact ? true : allowUpscale,
            suffix: "-resized",
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
                    detail: "\(from) → \(to) px · \(ByteFormat.savings(from: result.originalBytes, to: result.newBytes))"
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
