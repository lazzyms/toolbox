import SwiftUI
import ToolboxKit
import UniformTypeIdentifiers

struct PDFUnlockView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var password = ""
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
            runTitle: "Remove Password",
            canRun: !files.isEmpty && !password.isEmpty,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Password") {
                    // One password applies to the whole batch — the common case is
                    // a set of statements from the same issuer.
                    SecureField("The PDF's current password", text: $password)
                        .textFieldStyle(.roundedBorder)
                        .frame(maxWidth: 260)
                        .onSubmit(start)
                }

                DestinationPicker(location: $location)

                Label(
                    "Only works with the correct password. Nothing leaves your Mac.",
                    systemImage: "hand.raised.fill"
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        } content: {
            DropZone(
                prompt: "Drop password-protected PDFs here",
                allowedExtensions: ["pdf"],
                contentTypes: [.pdf],
                files: $files
            )

            if !files.isEmpty {
                FileList(files: $files)

                if encryptedCount < files.count {
                    Label(
                        "\(files.count - encryptedCount) of these have no password and will be skipped.",
                        systemImage: "info.circle"
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }

            if !outcomes.isEmpty {
                ResultsList(outcomes: outcomes)
            }
        }
    }

    private var encryptedCount: Int {
        files.filter { PDFUnlocker.isEncrypted($0) }.count
    }

    private func start() {
        guard !files.isEmpty, !password.isEmpty, !isRunning else { return }
        let inputs = files
        let secret = password
        let destination = location

        isRunning = true
        progress = 0
        outcomes = []

        Task {
            let results = await BatchRunner.run(inputs) { progressCount, total in
                Task { @MainActor in
                    progress = Double(progressCount) / Double(total)
                }
            } job: { url in
                let result = try PDFUnlocker.removePassword(
                    from: url, password: secret, to: destination
                )
                return JobOutcome(
                    input: url,
                    output: result.output,
                    detail: "Unlocked · \(result.pageCount) page\(result.pageCount == 1 ? "" : "s") → \(result.output.lastPathComponent)"
                )
            }

            await MainActor.run {
                outcomes = results
                isRunning = false
                progress = nil
                // Keep failures queued so a typo'd password can be retried
                // without re-dropping the files.
                files = results.filter { !$0.succeeded }.map(\.input)
            }
        }
    }
}
