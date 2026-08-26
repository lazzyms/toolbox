import SwiftUI
import ToolboxKit

struct PDFProtectView: View {
    let utility: Utility

    @State private var files: [URL] = []
    @State private var password = ""
    @State private var confirmation = ""
    @State private var ownerPassword = ""
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
            runTitle: "Protect PDFs",
            canRun: canRun,
            run: start
        ) {
            VStack(alignment: .leading, spacing: 14) {
                OptionRow(label: "Password") {
                    SecureField("Required to open the file", text: $password)
                        .textFieldStyle(.roundedBorder)
                }

                OptionRow(label: "Confirm") {
                    SecureField("Repeat the password", text: $confirmation)
                        .textFieldStyle(.roundedBorder)
                }

                if !confirmation.isEmpty && password != confirmation {
                    Text("Passwords don't match.")
                        .font(.caption)
                        .foregroundStyle(.red)
                }

                OptionRow(label: "Owner password") {
                    SecureField("Optional — defaults to the password above", text: $ownerPassword)
                        .textFieldStyle(.roundedBorder)
                }

                DestinationPicker(location: $location)

                VStack(alignment: .leading, spacing: 4) {
                    Text("A typo in this password is unrecoverable — check the confirmation carefully. The original files stay where they are; only the new “-protected” copies are secured.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text("macOS applies AES encryption (verified after writing; the result line shows what was applied).")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
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
    }

    private var canRun: Bool {
        !files.isEmpty
            && !isRunning
            && !password.isEmpty
            && password == confirmation
    }

    private func start() {
        guard canRun else { return }
        let inputs = files
        let pw = password
        let owner = ownerPassword.isEmpty ? nil : ownerPassword
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
                let output = try PDFProtector.apply(
                    password: pw, ownerPassword: owner, to: url, to: dest
                )
                let algorithm = PDFProtector.encryptionAlgorithm(of: output) ?? "unknown encryption"
                return JobOutcome(input: url, output: output, detail: "Protected (\(algorithm)) → \(output.lastPathComponent)")
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
