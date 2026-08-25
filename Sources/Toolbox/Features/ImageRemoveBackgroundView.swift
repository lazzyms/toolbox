import SwiftUI
import ToolboxKit

struct ImageRemoveBackgroundView: View {
    let utility: Utility

    @State private var files: [URL] = []
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
            runTitle: "Remove Background",
            canRun: !files.isEmpty,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                DestinationPicker(location: $location)

                Text(
                    "Finds the subject on-device with Vision and writes a transparent "
                        + "PNG next to each original. A file without a clear subject "
                        + "fails rather than producing a blank cutout."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(.leading, presentation.explanationInset)
                .fixedSize(horizontal: false, vertical: true)
            }
        } content: {
            DropZone(
                prompt: "Drop photos to cut out",
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
        let dest = location

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            // Segmentation is far heavier per file than a resize, so cap below
            // the core count instead of saturating every lane.
            let lanes = max(1, ProcessInfo.processInfo.activeProcessorCount / 2)
            let results = await BatchRunner.run(inputs, maxConcurrent: lanes) { done, total in
                Task { @MainActor in progress = Double(done) / Double(total) }
            } job: { url in
                let output = try ImageBackgroundRemover.run(url, location: dest)
                let size = ByteFormat.string(OutputNaming.fileSize(of: output))
                return JobOutcome(
                    input: url,
                    output: output,
                    detail: "\(output.lastPathComponent) · \(size)"
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
