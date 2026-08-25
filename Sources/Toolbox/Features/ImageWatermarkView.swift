import SwiftUI
import ToolboxKit
import UniformTypeIdentifiers

struct ImageWatermarkView: View {
    enum Mode: Hashable { case text, image }
    enum Sizing: Hashable { case percentOfWidth, points }
    enum AnchorChoice: Hashable { case centre, topLeft, topRight, bottomLeft, bottomRight, tiled

        var stampAnchor: StampAnchor {
            switch self {
            case .centre: return .center
            case .topLeft: return .topLeft
            case .topRight: return .topRight
            case .bottomLeft: return .bottomLeft
            case .bottomRight: return .bottomRight
            case .tiled: return .tiled(spacing: 60)
            }
        }
    }
    enum Colour: String, CaseIterable { case grey, black, red

        var cgColor: CGColor {
            switch self {
            case .grey: return CGColor(gray: 0.5, alpha: 1)
            case .black: return CGColor(gray: 0, alpha: 1)
            case .red: return CGColor(red: 0.85, green: 0.1, blue: 0.1, alpha: 1)
            }
        }
    }

    let utility: Utility

    @State private var files: [URL] = []
    @State private var mode: Mode = .text
    @State private var text = "DRAFT"
    @State private var logoURL: URL?
    @State private var showLogoPicker = false
    @State private var colour: Colour = .grey
    // Percent is the default because it's what keeps a mixed batch consistent:
    // an absolute size that reads well on photos vanishes on thumbnails.
    @State private var sizing: Sizing = .percentOfWidth
    @State private var sizePercent = 8.0
    @State private var sizePoints = 48.0
    @State private var opacity = 0.35
    @State private var rotation = -45.0
    @State private var imageScale = 0.4
    @State private var anchor: AnchorChoice = .centre
    @State private var quality = 0.8
    @State private var location: OutputLocation = .alongsideInput
    @State private var outcomes: [JobOutcome] = []
    @State private var isRunning = false
    @State private var progress: Double?

    private var anchors: [(String, AnchorChoice)] {
        [
            ("Centre", .centre), ("Top left", .topLeft), ("Top right", .topRight),
            ("Bottom left", .bottomLeft), ("Bottom right", .bottomRight), ("Tiled", .tiled),
        ]
    }

    var body: some View {
        ToolScaffold(
            utility: utility,
            fileCount: files.count,
            isRunning: isRunning,
            progress: progress,
            runTitle: "Add Watermark",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Stamp") {
                    Picker("", selection: $mode) {
                        Text("Text").tag(Mode.text)
                        Text("Image").tag(Mode.image)
                    }
                    .pickerStyle(.segmented)
                }

                if mode == .text {
                    OptionRow(label: "Text") {
                        TextField("DRAFT", text: $text)
                            .textFieldStyle(.roundedBorder)
                    }
                    OptionRow(label: "Colour") {
                        Picker("", selection: $colour) {
                            ForEach(Colour.allCases, id: \.self) { c in
                                Text(c.rawValue.capitalized).tag(c)
                            }
                        }
                        .pickerStyle(.segmented)
                    }
                    OptionRow(label: "Size") {
                        Picker("", selection: $sizing) {
                            Text("% of width").tag(Sizing.percentOfWidth)
                            Text("Points").tag(Sizing.points)
                        }
                        .pickerStyle(.segmented)
                    }
                    if sizing == .percentOfWidth {
                        OptionRow(label: "Size") {
                            Slider(value: $sizePercent, in: 1...30, step: 1) {
                                Text("\(Int(sizePercent))% of width")
                            } minimumValueLabel: { Text("1%") } maximumValueLabel: { Text("30%") }
                        }
                    } else {
                        OptionRow(label: "Size") {
                            Stepper("\(Int(sizePoints)) pt", value: $sizePoints, in: 8...200, step: 2)
                        }
                    }
                    OptionRow(label: "Rotation") {
                        Slider(value: $rotation, in: -90...90, step: 15) {
                            Text("\(Int(rotation))°")
                        } minimumValueLabel: { Text("-90°") } maximumValueLabel: { Text("90°") }
                    }
                } else {
                    OptionRow(label: "Logo") {
                        HStack {
                            Text(logoURL?.lastPathComponent ?? "None chosen")
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Spacer()
                            Button("Choose…") { showLogoPicker = true }
                        }
                    }
                    OptionRow(label: "Width") {
                        Slider(value: $imageScale, in: 0.05...1.0) {
                            Text("\(Int(imageScale * 100))% of image")
                        } minimumValueLabel: { Text("5%") } maximumValueLabel: { Text("100%") }
                    }
                }

                OptionRow(label: "Opacity") {
                    Slider(value: $opacity, in: 0.05...1.0) {
                        Text("\(Int(opacity * 100))%")
                    } minimumValueLabel: { Text("5%") } maximumValueLabel: { Text("100%") }
                }

                OptionRow(label: "Position") {
                    Picker("", selection: $anchor) {
                        ForEach(anchors, id: \.0) { name, choice in
                            Text(name).tag(choice)
                        }
                    }
                }

                OptionRow(label: "Quality") {
                    VStack(alignment: .leading, spacing: 2) {
                        Slider(value: $quality, in: 0.3...1.0)
                            .frame(maxWidth: 220)
                        // Only lossy outputs (a JPEG or HEIC input kept as-is)
                        // consume the setting; PNG results ignore it.
                        Text("Applies when the output stays JPEG or HEIC.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                DestinationPicker(location: $location)
            }
        } content: {
            DropZone(
                prompt: "Drop images here",
                allowedExtensions: ImageFormat.readableExtensions,
                contentTypes: [],
                files: $files
            )

            if !files.isEmpty {
                FileList(files: $files)
            }

            if !outcomes.isEmpty {
                ResultsList(outcomes: outcomes)
            }
        }
        .fileImporter(
            isPresented: $showLogoPicker,
            allowedContentTypes: [.png],
            allowsMultipleSelection: false
        ) { result in
            if case .success(let urls) = result, let url = urls.first {
                logoURL = url
            }
        }
    }

    private var canRun: Bool {
        guard !files.isEmpty, !isRunning else { return false }
        switch mode {
        case .text: return !text.trimmingCharacters(in: .whitespaces).isEmpty
        case .image: return logoURL != nil
        }
    }

    private func start() {
        guard canRun else { return }
        let inputs = files
        let fontSize: StampSize = switch sizing {
        case .percentOfWidth: .fraction(sizePercent / 100)
        case .points: .points(sizePoints)
        }
        let options: ImageWatermarkOptions
        switch mode {
        case .text:
            options = ImageWatermarkOptions(
                content: .text(text),
                fontSize: fontSize,
                color: colour.cgColor,
                opacity: opacity,
                rotationDegrees: rotation,
                anchor: anchor.stampAnchor
            )
        case .image:
            guard let url = logoURL else { return }
            options = ImageWatermarkOptions(
                content: .image(url),
                opacity: opacity,
                rotationDegrees: 0,
                anchor: anchor.stampAnchor,
                imageScale: imageScale
            )
        }
        let dest = location
        let quality = quality

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            let results = await BatchRunner.run(inputs) { progressCount, total in
                Task { @MainActor in
                    progress = Double(progressCount) / Double(total)
                }
            } job: { url in
                let output = try ImageWatermarker.apply(options, to: url, destination: dest, quality: quality)
                return JobOutcome(input: url, output: output, detail: "Watermarked → \(output.lastPathComponent)")
            }

            await MainActor.run {
                outcomes = results
                isRunning = false
                progress = nil
                files = results.filter { !$0.succeeded }.map(\.input)
            }
        }
    }
}
