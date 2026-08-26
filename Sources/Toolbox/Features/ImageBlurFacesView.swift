import SwiftUI
import ToolboxKit

struct ImageBlurFacesView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var radius = 12.0
    @State private var quality = 0.85
    @State private var location: OutputLocation = .alongsideInput
    @State private var outcomes: [JobOutcome] = []
    @State private var isRunning = false
    @State private var progress: Double?

    @Environment(\.toolPresentation) private var presentation

    private var options: ImageFaceBlurrer.Options {
        ImageFaceBlurrer.Options(radius: radius, quality: quality)
    }

    var body: some View {
        ToolScaffold(
            utility: utility,
            fileCount: files.count,
            isRunning: isRunning,
            progress: progress,
            runTitle: "Blur Faces",
            canRun: !files.isEmpty && radius > 0,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Strength") {
                    Slider(value: $radius, in: 2...60)
                        .frame(maxWidth: 220)
                    Text("\(Int(radius)) px")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(width: 52, alignment: .trailing)
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

                // A privacy tool that quietly under-delivers is worse than
                // none: detection misses profiles, hands and small background
                // faces, so the count in the results is "found", not "all".
                Text("Faces are detected on this Mac and never uploaded — but detection can miss faces. Check every result before sharing it.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } content: {
            DropZone(
                prompt: "Drop images to blur faces",
                allowedExtensions: ImageFormat.readableExtensions,
                contentTypes: [],
                files: $files
            )

            if !files.isEmpty { FileList(files: $files) }
            if !outcomes.isEmpty { ResultsList(outcomes: outcomes) }
        }
    }

    private func start() {
        guard !files.isEmpty, radius > 0, !isRunning else { return }
        let inputs = files
        // The batch runs against a snapshot: dragging a slider mid-run must
        // not change what the remaining files receive.
        let settings = options
        let destination = location

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            let results = await BatchRunner.run(inputs) { done, total in
                Task { @MainActor in progress = Double(done) / Double(total) }
            } job: { url in
                // The face count goes in the detail line on purpose: "found
                // 2 faces" lets a user with five people in frame spot that
                // three went unblurred.
                let result = try ImageFaceBlurrer.run(url, options: settings, location: destination)
                let noun = result.faceCount == 1 ? "face" : "faces"
                return JobOutcome(
                    input: url,
                    output: result.output,
                    detail: "Found \(result.faceCount) \(noun) — check before sharing"
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
