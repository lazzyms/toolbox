import SwiftUI
import ToolboxKit

struct CropView: View {
    let utility: Utility

    private enum Mode: String, CaseIterable, Identifiable {
        case aspect, numeric
        var id: String { rawValue }
        var title: String {
            switch self {
            case .aspect: return "Aspect ratio"
            case .numeric: return "Pixels"
            }
        }
    }

    private struct Ratio: Identifiable {
        let title: String
        let width: Int
        let height: Int
        var id: String { title }
    }

    private static let ratios: [Ratio] = [
        Ratio(title: "1:1", width: 1, height: 1),
        Ratio(title: "4:3", width: 4, height: 3),
        Ratio(title: "16:9", width: 16, height: 9),
        Ratio(title: "Custom", width: 0, height: 0),
    ]
    private static let customRatioIndex = ratios.count - 1

    @State private var files: [URL] = []
    @State private var mode: Mode = .aspect
    @State private var ratioIndex = 0
    @State private var customWidth = "16"
    @State private var customHeight = "9"
    @State private var anchor: CropAnchor = .center
    @State private var cropX = "0"
    @State private var cropY = "0"
    @State private var cropWidth = "100"
    @State private var cropHeight = "100"
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
            runTitle: "Crop",
            canRun: !files.isEmpty && crop.isValid,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Mode") {
                    Picker("Mode", selection: $mode) {
                        ForEach(Mode.allCases) { Text($0.title).tag($0) }
                    }
                    .labelsHidden()
                    .optionPickerStyle(presentation)
                }

                switch mode {
                case .aspect:
                    OptionRow(label: "Ratio") {
                        Picker("Ratio", selection: $ratioIndex) {
                            ForEach(Array(Self.ratios.enumerated()), id: \.element.id) { index, ratio in
                                Text(ratio.title).tag(index)
                            }
                        }
                        .labelsHidden()
                        .optionPickerStyle(presentation)
                    }

                    if ratioIndex == Self.customRatioIndex {
                        OptionRow(label: "Custom") {
                            pixelField($customWidth, label: "Width")
                            Text(":").foregroundStyle(.secondary)
                            pixelField($customHeight, label: "Height")
                        }
                    }

                    OptionRow(label: "Anchor") {
                        AnchorGrid(anchor: $anchor)
                    }
                    Text("Crops to the largest rectangle of this ratio that fits. Anchor pins where it sits — square a photo from the centre for a profile picture, or frame a banner against the top edge.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.leading, presentation.explanationInset)
                        .fixedSize(horizontal: false, vertical: true)

                case .numeric:
                    OptionRow(label: "Crop rect") {
                        pixelField($cropX, label: "X")
                        Text(",").foregroundStyle(.secondary)
                        pixelField($cropY, label: "Y")
                        Text("to").font(.caption).foregroundStyle(.secondary)
                        pixelField($cropWidth, label: "Width")
                        Text("×").foregroundStyle(.secondary)
                        pixelField($cropHeight, label: "Height")
                    }
                    Text("Pixels from the top-left of the image as shown, the same rect for the whole batch. Files it doesn't fit are failed with a reason, not silently altered.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.leading, presentation.explanationInset)
                        .fixedSize(horizontal: false, vertical: true)
                }

                DestinationPicker(location: $location)
            }
        } content: {
            DropZone(
                prompt: "Drop images to crop",
                allowedExtensions: ImageFormat.readableExtensions,
                contentTypes: [],
                files: $files
            )

            if !files.isEmpty { FileList(files: $files) }
            if !outcomes.isEmpty { ResultsList(outcomes: outcomes) }
        }
    }

    private var crop: CropOptions {
        switch mode {
        case .aspect:
            let ratio = Self.ratios[ratioIndex]
            let width = ratio.width > 0 ? ratio.width : Int(customWidth) ?? 0
            let height = ratio.height > 0 ? ratio.height : Int(customHeight) ?? 0
            return .aspect(width: width, height: height, anchor: anchor)
        case .numeric:
            return .rect(
                x: Int(cropX) ?? 0,
                y: Int(cropY) ?? 0,
                width: Int(cropWidth) ?? 0,
                height: Int(cropHeight) ?? 0
            )
        }
    }

    private func pixelField(_ binding: Binding<String>, label: String) -> some View {
        TextField(label, text: binding)
            .textFieldStyle(.roundedBorder)
            .frame(width: 56)
            .multilineTextAlignment(.trailing)
            .accessibilityLabel(label)
            .onSubmit(start)
    }

    private func start() {
        guard !files.isEmpty, crop.isValid, !isRunning else { return }
        let inputs = files
        let crop = crop
        let options = ImageProcessor.Options(
            format: nil,
            quality: 0.9,
            operations: [crop.operation],
            suffix: "-cropped",
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
                // Fit is per-image, so the same batch can mix sizes. Checking
                // here — before processing — is what turns a smaller image into
                // a named failure instead of a silent partial crop.
                if let size = CropOptions.pixelSize(of: url),
                   let error = crop.validationError(for: size) {
                    throw error
                }
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

/// A 3×3 grid of anchor positions, with the selected cell highlighted.
private struct AnchorGrid: View {
    @Binding var anchor: CropAnchor

    private static let layout: [[CropAnchor]] = [
        [.topLeft, .top, .topRight],
        [.left, .center, .right],
        [.bottomLeft, .bottom, .bottomRight],
    ]

    var body: some View {
        VStack(spacing: 3) {
            ForEach(0..<3, id: \.self) { row in
                HStack(spacing: 3) {
                    ForEach(0..<3, id: \.self) { column in
                        let cell = Self.layout[row][column]
                        Button {
                            anchor = cell
                        } label: {
                            Rectangle()
                                .fill(cell == anchor
                                    ? Color.accentColor
                                    : Color.secondary.opacity(0.25))
                                .frame(width: 14, height: 14)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(cell.rawValue)
                    }
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Anchor position")
    }
}
