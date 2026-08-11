import Foundation
import Sparkle
import SwiftUI

/// Owns the Sparkle updater and exposes just enough state for the UI.
///
/// Sparkle handles the security-critical work: it verifies every download
/// against the Ed25519 public key in Info.plist (`SUPublicEDKey`) before
/// anything is extracted or executed. A compromised GitHub account or a
/// tampered download therefore cannot push code to installed copies — the
/// signature check fails and the update is discarded.
@MainActor
final class UpdateController: NSObject, ObservableObject {

    /// Enabled only when the app is a proper bundle with an appcast URL. A
    /// `swift run` binary has neither, and Sparkle would log errors on launch.
    @Published private(set) var isAvailable = false
    @Published private(set) var canCheck = false
    @Published var lastCheckDate: Date?

    private var controller: SPUStandardUpdaterController?

    override init() {
        super.init()

        // Bundled builds have SUFeedURL; `swift run` does not.
        guard Bundle.main.object(forInfoDictionaryKey: "SUFeedURL") != nil else {
            isAvailable = false
            return
        }

        // startingUpdater: true begins the scheduled-check timer. On first launch
        // Sparkle asks the user whether to check automatically and stores the
        // answer, so no network request happens before they have agreed.
        let controller = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: self,
            userDriverDelegate: nil
        )
        self.controller = controller
        isAvailable = true
        canCheck = true
        lastCheckDate = controller.updater.lastUpdateCheckDate
    }

    /// Whether Sparkle checks on its own schedule. Bound to the Settings toggle.
    var automaticallyChecksForUpdates: Bool {
        get { controller?.updater.automaticallyChecksForUpdates ?? false }
        set {
            controller?.updater.automaticallyChecksForUpdates = newValue
            objectWillChange.send()
        }
    }

    var currentVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString")
            as? String ?? "—"
    }

    /// User-initiated check. Sparkle reports "you're up to date" itself, so the
    /// UI doesn't need to.
    func checkForUpdates() {
        controller?.checkForUpdates(nil)
        lastCheckDate = controller?.updater.lastUpdateCheckDate
    }

    /// Backs the "Check for Updates…" menu item's enabled state.
    var canCheckForUpdates: Bool {
        controller?.updater.canCheckForUpdates ?? false
    }
}

extension UpdateController: SPUUpdaterDelegate {
    nonisolated func updater(_ updater: SPUUpdater, didFinishLoading appcast: SUAppcast) {
        Task { @MainActor in
            self.lastCheckDate = updater.lastUpdateCheckDate
        }
    }

    /// Sparkle surfaces its own error dialogs; this only keeps our state honest.
    nonisolated func updater(_ updater: SPUUpdater, didAbortWithError error: Error) {
        Task { @MainActor in
            self.lastCheckDate = updater.lastUpdateCheckDate
        }
    }
}
