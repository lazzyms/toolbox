import SwiftUI

/// Settings window: where the app lives, and update preferences.
struct SettingsView: View {
    @ObservedObject var settings: AppSettings
    @ObservedObject var updates: UpdateController

    var body: some View {
        TabView {
            GeneralSettings(settings: settings)
                .tabItem { Label("General", systemImage: "gearshape") }

            UpdateSettings(updates: updates)
                .tabItem { Label("Updates", systemImage: "arrow.down.circle") }
        }
        .frame(width: 440)
    }
}

/// Menu bar / Dock presence and login item.
struct GeneralSettings: View {
    @ObservedObject var settings: AppSettings

    /// `SMAppService.register()` can fail (an unapproved login item, a bundle
    /// macOS won't accept); showing why beats silently reverting the toggle.
    @State private var loginItemError: String?

    var body: some View {
        Form {
            Section {
                Toggle("Show Toolbox in the menu bar", isOn: $settings.showsMenuBarIcon)

                Toggle("Hide Dock icon", isOn: $settings.hidesDockIcon)
                    .help("Toolbox keeps running in the menu bar with no Dock icon and no app switcher entry.")

                if settings.hidesDockIcon {
                    Text("Toolbox stays reachable from the menu bar icon. Open the main window from there whenever you need it.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            } header: {
                Text("Appearance")
            } footer: {
                // Explains why the menu bar toggle can flip itself back on.
                Text("Hiding the Dock icon keeps the menu bar icon switched on — otherwise there would be no way to reach Toolbox again.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if settings.isLoginItemAvailable {
                Section {
                    Toggle(
                        "Open Toolbox at login",
                        isOn: Binding(
                            get: { settings.launchesAtLogin },
                            set: { enabled in
                                let error = settings.setLaunchesAtLogin(enabled)
                                loginItemError = error?.localizedDescription
                            }
                        )
                    )

                    if let loginItemError {
                        Label(loginItemError, systemImage: "exclamationmark.triangle")
                            .font(.caption)
                            .foregroundStyle(.orange)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                } header: {
                    Text("Startup")
                }
            }

            Section {
                Toggle("Share anonymous install analytics", isOn: $settings.analyticsEnabled)
                    .help("Helps measure Toolbox adoption. No filenames, file contents, paths, or hardware identifiers are collected.")
            } header: {
                Text("Privacy")
            } footer: {
                Text("Enabled for new installs to measure anonymous app installations and launches. You can turn this off at any time.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Section {
                HStack(spacing: 14) {
                    Image("BuyMeACoffeeQR", bundle: .module)
                        .resizable()
                        .interpolation(.none)
                        .scaledToFit()
                        .frame(width: 84, height: 84)
                        .accessibilityLabel("Buy Me a Coffee donation QR code")

                    VStack(alignment: .leading, spacing: 4) {
                        Text("Enjoying Toolbox?")
                            .font(.subheadline.weight(.medium))
                        Text("If you’d like to support development, scan the QR code to buy me a coffee.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            } header: {
                Text("Support Toolbox")
            }
        }
        .formStyle(.grouped)
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// Update preferences.
struct UpdateSettings: View {
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
        .fixedSize(horizontal: false, vertical: true)
    }
}
