import SwiftUI
import ToolboxKit
import UniformTypeIdentifiers

struct PDFWatermarkView: View {
    enum Mode: Hashable { case text, image }
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
    @State private var imageURL: URL?
    @State private var showImagePicker = false
    @State private var colour: Colour = .grey
    @State private var opacity = 0.3
    @State private var rotation = -45.0
    @State private var fontSize = 48.0
    @State private var imageScale = 0.4
    @State private var anchor: AnchorChoice = .centre
    @State private var under = false
    @State private var pageRange = ""
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
                    OptionRow(label: "Font size") {
                        Stepper("\(Int(fontSize)) pt", value: $fontSize, in: 8...200, step: 2)
                    }
                } else {
                    OptionRow(label: "Image") {
                        HStack {
                            Text(imageURL?.lastPathComponent ?? "None chosen")
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Spacer()
                            Button("Choose…") { showImagePicker = true }
                        }
                    }
                    OptionRow(label: "Width") {
                        Slider(value: $imageScale, in: 0.05...1.0) {
                            Text("\(Int(imageScale * 100))% of page")
                        } minimumValueLabel: { Text("5%") } maximumValueLabel: { Text("100%") }
                    }
                }

                if mode == .text {
                    OptionRow(label: "Rotation") {
                        Slider(value: $rotation, in: -90...90, step: 15) {
                            Text("\(Int(rotation))°")
                        } minimumValueLabel: { Text("-90°") } maximumValueLabel: { Text("90°") }
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

                Toggle(isOn: $under) {
                    VStack(alignment: .leading) {
                        Text("Under page content")
                        Text("Keeps body text readable — good for letterheads.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                OptionRow(label: "Page range") {
                    TextField("e.g. 1-3,5,7-", text: $pageRange)
                        .textFieldStyle(.roundedBorder)
                }

                DestinationPicker(location: $location)
            }
        } content: {
            DropZone(
                prompt: "Drop PDFs here",
                allowedExtensions: ["pdf"],
                contentTypes: [.pdf],
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
            isPresented: $showImagePicker,
            allowedContentTypes: [.png, .jpeg],
            allowsMultipleSelection: false
        ) { result in
            if case .success(let urls) = result, let url = urls.first {
                imageURL = url
            }
        }
    }

    private var resolvedAnchor: StampAnchor {
        anchor.stampAnchor
    }

    private var canRun: Bool {
        guard !files.isEmpty, !isRunning else { return false }
        switch mode {
        case .text: return !text.trimmingCharacters(in: .whitespaces).isEmpty
        case .image: return imageURL != nil
        }
    }

    private func start() {
        guard canRun else { return }
        let inputs = files
        let options: WatermarkOptions
        switch mode {
        case .text:
            options = WatermarkOptions(
                content: .text(text),
                fontSize: fontSize,
                color: colour.cgColor,
                opacity: opacity,
                rotationDegrees: rotation,
                anchor: resolvedAnchor,
                underContent: under
            )
        case .image:
            guard let url = imageURL else { return }
            options = WatermarkOptions(
                content: .image(url),
                opacity: opacity,
                anchor: resolvedAnchor,
                underContent: under,
                imageScale: imageScale
            )
        }
        let range = pageRange.isEmpty ? nil : pageRange
        let dest = location

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            let results = await BatchRunner.run(inputs) { progressCount, total in
                Task { @MainActor in
                    progress = Double(progressCount) / Double(total)
                }
            } job: { url in
                let output = try PDFWatermarker.apply(options, to: url, pageRangeText: range, to: dest)
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
