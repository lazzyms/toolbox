import SwiftUI
import ToolboxKit

struct ImageToneView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var brightness = 0.0
    @State private var contrast = 1.0
    @State private var saturation = 1.0
    @State private var exposure = 0.0
    @State private var temperature = 6500.0
    @State private var tint = 0.0
    @State private var grayscale = false
    @State private var sepia = false
    @State private var invert = false
    @State private var autoEnhance = false
    @State private var quality = 0.85
    @State private var location: OutputLocation = .alongsideInput
    @State private var outcomes: [JobOutcome] = []
    @State private var isRunning = false
    @State private var progress: Double?

    @Environment(\.toolPresentation) private var presentation

    private var options: ImageToneAdjuster.Options {
        ImageToneAdjuster.Options(
            brightness: brightness,
            contrast: contrast,
            saturation: saturation,
            exposure: exposure,
            temperature: temperature,
            tint: tint,
            grayscale: grayscale,
            sepia: sepia,
            invert: invert,
            autoEnhance: autoEnhance,
            quality: quality
        )
    }

    /// The verb describing what a run will do, shown per file in the results.
    private var summary: String {
        var parts: [String] = []
        if autoEnhance { parts.append("Auto-enhance") }
        if brightness != 0 { parts.append("Brightness \(signed(brightness, digits: 0))%") }
        if contrast != 1 { parts.append("Contrast ×\(contrast.formatted(.number.precision(.fractionLength(2))))") }
        if saturation == 0 {
            parts.append("Grayscale")
        } else if saturation != 1 {
            parts.append("Saturation ×\(saturation.formatted(.number.precision(.fractionLength(2))))")
        }
        if exposure != 0 { parts.append("Exposure \(signed(exposure, digits: 1)) EV") }
        if temperature != 6500 { parts.append("\(Int(temperature)) K") }
        if tint != 0 { parts.append("Tint \(Int(tint))") }
        if grayscale { parts.append("Grayscale") }
        if sepia { parts.append("Sepia") }
        if invert { parts.append("Invert") }
        return parts.joined(separator: ", ")
    }

    private func signed(_ value: Double, digits: Int) -> String {
        let sign = value < 0 ? "-" : "+"
        return sign + String(format: "%.\(digits)f", abs(value))
    }

    var body: some View {
        ToolScaffold(
            utility: utility,
            fileCount: files.count,
            isRunning: isRunning,
            progress: progress,
            runTitle: "Apply",
            canRun: !files.isEmpty && options.isActive,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Brightness") {
                    Slider(value: $brightness, in: -1...1)
                        .frame(maxWidth: 220)
                    Text(signed(brightness, digits: 0) + "%")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(width: 44, alignment: .trailing)
                }

                OptionRow(label: "Contrast") {
                    Slider(value: $contrast, in: 0...2)
                        .frame(maxWidth: 220)
                    Text(contrast.formatted(.number.precision(.fractionLength(2))) + "×")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(width: 44, alignment: .trailing)
                }

                OptionRow(label: "Saturation") {
                    Slider(value: $saturation, in: 0...2)
                        .frame(maxWidth: 220)
                    Text(saturation.formatted(.number.precision(.fractionLength(2))) + "×")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(width: 44, alignment: .trailing)
                }

                OptionRow(label: "Exposure") {
                    Slider(value: $exposure, in: -3...3)
                        .frame(maxWidth: 220)
                    Text(signed(exposure, digits: 1) + " EV")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(width: 60, alignment: .trailing)
                }

                OptionRow(label: "Warmth") {
                    Slider(value: $temperature, in: 3000...10000)
                        .frame(maxWidth: 220)
                    Text("\(Int(temperature)) K")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(width: 52, alignment: .trailing)
                }

                OptionRow(label: "Tint") {
                    Slider(value: $tint, in: -150...150)
                        .frame(maxWidth: 220)
                    Text(Int(tint).formatted())
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(width: 36, alignment: .trailing)
                }

                OptionRow(label: "Presets") {
                    Toggle("Grayscale", isOn: $grayscale)
                    Toggle("Sepia", isOn: $sepia)
                    Toggle("Invert", isOn: $invert)
                }

                OptionRow(label: "Auto") {
                    Toggle("Auto-enhance", isOn: $autoEnhance)
                }

                OptionRow(label: "Quality") {
                    Slider(value: $quality, in: 0.5...1)
                        .frame(maxWidth: 220)
                    Text("\(Int(quality * 100))%")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(width: 38, alignment: .trailing)
                }

                DestinationPicker(location: $location)
            }
        } content: {
            DropZone(
                prompt: "Drop images to adjust",
                allowedExtensions: ImageFormat.readableExtensions,
                contentTypes: [],
                files: $files
            )

            if !files.isEmpty { FileList(files: $files) }
            if !outcomes.isEmpty { ResultsList(outcomes: outcomes) }
        }
    }

    private func start() {
        guard !files.isEmpty, options.isActive, !isRunning else { return }
        let inputs = files
        // The batch runs against a snapshot: dragging a slider mid-run must not
        // change what the remaining files receive.
        let settings = options
        let description = summary.isEmpty ? "No adjustments" : summary
        let destination = location

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            let results = await BatchRunner.run(inputs) { done, total in
                Task { @MainActor in progress = Double(done) / Double(total) }
            } job: { url in
                let output = try ImageToneAdjuster.run(url, options: settings, location: destination)
                return JobOutcome(input: url, output: output, detail: description)
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
