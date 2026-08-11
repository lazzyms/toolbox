import SwiftUI

/// Settings window — currently just update preferences.
struct SettingsView: View {
    @ObservedObject var updates: UpdateController

    private var lastChecked: String {
        guard let date = updates.lastCheckDate else { return "Never" }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        return formatter.localizedString(for: date, relativeTo: Date())
    }

    var body: some View {
        Form {
            Section {
                if updates.isAvailable {
                    Toggle(
                        "Check for updates automatically",
                        isOn: Binding(
                            get: { updates.automaticallyChecksForUpdates },
                            set: { updates.automaticallyChecksForUpdates = $0 }
                        )
                    )

                    HStack {
                        Text("Last checked")
                            .foregroundStyle(.secondary)
                        Spacer()
                        Text(lastChecked)
                            .foregroundStyle(.secondary)
                    }

                    HStack {
                        Spacer()
                        Button("Check Now") { updates.checkForUpdates() }
                            .disabled(!updates.canCheckForUpdates)
                    }
                } else {
                    // Shown for `swift run` builds, which have no appcast.
                    Label(
                        "Updates are only available in the installed app.",
                        systemImage: "info.circle"
                    )
                    .foregroundStyle(.secondary)
                }
            } header: {
                Text("Updates")
            } footer: {
                Text("Toolbox \(updates.currentVersion). Updates are downloaded from GitHub and verified with a cryptographic signature before installing. Your files are never uploaded.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .formStyle(.grouped)
        .frame(width: 440)
        .fixedSize(horizontal: false, vertical: true)
    }
}
