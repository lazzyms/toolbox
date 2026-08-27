import SwiftUI
import FirebaseCore
import Foundation

@main
struct ToolboxApp: App {
    // Created once for the app's lifetime; Sparkle's scheduled checks depend on
    // the updater outliving any single window.
    @StateObject private var updates = UpdateController()

    // Shared instance: it owns the process-wide activation policy, and the app
    // delegate has to read the same values before the first window appears.
    @StateObject private var settings = AppSettings.shared

    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    init() {
        guard let resources = Bundle.main.url(forResource: "Toolbox_Toolbox", withExtension: "bundle"),
              let bundle = Bundle(url: resources),
              let url = bundle.url(forResource: "GoogleService-Info", withExtension: "plist"),
              let values = NSDictionary(contentsOf: url),
              let appID = values["GOOGLE_APP_ID"] as? String,
              let senderID = values["GCM_SENDER_ID"] as? String else { return }
        let options = FirebaseOptions(googleAppID: appID, gcmSenderID: senderID)
        options.apiKey = values["API_KEY"] as? String
        options.projectID = values["PROJECT_ID"] as? String
        options.storageBucket = values["STORAGE_BUCKET"] as? String
        FirebaseApp.configure(options: options)
        AnalyticsManager.shared.configure(enabled: AppSettings.shared.analyticsEnabled)
    }

    var body: some Scene {
        Window("Toolbox", id: MainWindow.id) {
            ContentView()
                .frame(minWidth: 780, minHeight: 560)
        }
        .windowResizability(.contentMinSize)
        .commands {
            CommandGroup(replacing: .newItem) {}

            // Sits in the app menu next to About, where macOS users expect it.
            CommandGroup(after: .appInfo) {
                if updates.isAvailable {
                    Button("Check for Updates…") { updates.checkForUpdates() }
                        .disabled(!updates.canCheckForUpdates)
                }
            }
        }

        // The same tools in a popover, so the app can drop out of the Dock and
        // still be fully usable. `isInserted` adds and removes the status item
        // live — no relaunch when the setting is toggled.
        MenuBarExtra(isInserted: $settings.showsMenuBarIcon) {
            MenuBarPanel(settings: settings, updates: updates)
        } label: {
            Image(systemName: "hammer.fill")
        }
        .menuBarExtraStyle(.window)

        Settings {
            SettingsView(settings: settings, updates: updates)
        }
    }
}

struct ContentView: View {
    @State private var selection: Utility.ID? = Utility.all.first?.id

    private var selected: Utility? {
        Utility.all.first { $0.id == selection }
    }

    var body: some View {
        NavigationSplitView {
            List(selection: $selection) {
                ForEach(Utility.Category.allCases) { category in
                    Section(category.rawValue) {
                        ForEach(Utility.inCategory(category)) { utility in
                            HStack(spacing: 10) {
                                Image(systemName: utility.symbol)
                                    .foregroundStyle(utility.tint)
                                    .frame(width: 20)
                                Text(utility.title)
                                    .lineLimit(1)
                            }
                            .tag(utility.id)
                        }
                    }
                }
            }
            .navigationSplitViewColumnWidth(min: 210, ideal: 230, max: 280)
        } detail: {
            if let selected {
                // Identity keyed on the utility so switching tools resets each
                // pane's file queue and results instead of leaking state across.
                selected.makeView()
                    .id(selected.id)
            } else {
                ContentUnavailableView(
                    "Pick a utility",
                    systemImage: "square.grid.2x2",
                    description: Text("Choose a tool from the sidebar to get started.")
                )
            }
        }
    }
}
