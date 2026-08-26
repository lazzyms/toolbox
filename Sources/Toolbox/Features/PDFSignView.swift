import SwiftUI
import ToolboxKit
import UniformTypeIdentifiers

struct PDFSignView: View {
    enum Mode: Hashable { case image, typed }
    enum AnchorChoice: String, CaseIterable {
        case topLeft, top, topRight, left, centre, right, bottomLeft, bottom, bottomRight

        var stampAnchor: StampAnchor {
            switch self {
            case .topLeft: return .topLeft
            case .top: return .top
            case .topRight: return .topRight
            case .left: return .left
            case .centre: return .center
            case .right: return .right
            case .bottomLeft: return .bottomLeft
            case .bottom: return .bottom
            case .bottomRight: return .bottomRight
            }
        }

        var label: String {
            switch self {
            case .topLeft: return "Top left"
            case .top: return "Top centre"
            case .topRight: return "Top right"
            case .left: return "Middle left"
            case .centre: return "Centre"
            case .right: return "Middle right"
            case .bottomLeft: return "Bottom left"
            case .bottom: return "Bottom centre"
            case .bottomRight: return "Bottom right"
            }
        }
    }

    let utility: Utility

    @State private var files: [URL] = []
    @State private var mode: Mode = .image
    @State private var imageURL: URL?
    @State private var showImagePicker = false
    @State private var typedName = ""
    @State private var fontSize = 36.0
    @State private var widthFraction = 0.28
    @State private var anchor: AnchorChoice = .bottomRight
    @State private var inset = 24.0
    @State private var pageRange = ""
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
            runTitle: "Sign PDF",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                Text("Adds a visible signature — an image or a typed name flattened into the page. This is not cryptographic signing: it carries no certificate and proves nothing about authenticity.")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                OptionRow(label: "Signature") {
                    Picker("", selection: $mode) {
                        Text("Image").tag(Mode.image)
                        Text("Type name").tag(Mode.typed)
                    }
                    .pickerStyle(.segmented)
                }

                if mode == .image {
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
                        Slider(value: $widthFraction, in: 0.05...0.8) {
                            Text("\(Int(widthFraction * 100))% of page")
                        } minimumValueLabel: { Text("5%") } maximumValueLabel: { Text("80%") }
                    }
                } else {
                    OptionRow(label: "Name") {
                        TextField("Your name", text: $typedName)
                            .textFieldStyle(.roundedBorder)
                    }
                    OptionRow(label: "Size") {
                        Stepper("\(Int(fontSize)) pt", value: $fontSize, in: 12...120, step: 4)
                    }
                }

                OptionRow(label: "Position") {
                    Picker("", selection: $anchor) {
                        ForEach(AnchorChoice.allCases, id: \.self) { choice in
                            Text(choice.label).tag(choice)
                        }
                    }
                }

                OptionRow(label: "Edge margin") {
                    Stepper("\(Int(inset)) pt", value: $inset, in: 0...144, step: 4)
                }

                OptionRow(label: "Page range") {
                    TextField("e.g. 2 or 1-3,7 — empty signs every page", text: $pageRange)
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

    private var canRun: Bool {
        guard !files.isEmpty, !isRunning else { return false }
        switch mode {
        case .image: return imageURL != nil
        case .typed: return !typedName.trimmingCharacters(in: .whitespaces).isEmpty
        }
    }

    private func start() {
        guard canRun else { return }
        let inputs = files
        let options: SignOptions
        switch mode {
        case .image:
            guard let url = imageURL else { return }
            options = SignOptions(
                content: .image(url),
                anchor: anchor.stampAnchor,
                inset: inset,
                widthFraction: widthFraction
            )
        case .typed:
            options = SignOptions(
                content: .typedName(typedName),
                anchor: anchor.stampAnchor,
                inset: inset,
                fontSize: fontSize
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
                let output = try PDFSigner.apply(options, to: url, pageRangeText: range, to: dest)
                return JobOutcome(input: url, output: output, detail: "Signed → \(output.lastPathComponent)")
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
