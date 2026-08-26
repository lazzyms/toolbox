import AppKit
import ServiceManagement
import SwiftUI

/// Where the app shows itself: menu bar, Dock, or both.
///
/// Single shared instance because the thing it controls — the process's
/// activation policy — is itself global; two owners could disagree about whether
/// the Dock icon is showing. Persisted in `UserDefaults` so the choice survives
/// a relaunch, and read back before the first window appears.
@MainActor
final class AppSettings: ObservableObject {
    static let shared = AppSettings()

    private enum Key {
        static let showsMenuBarIcon = "showsMenuBarIcon"
        static let hidesDockIcon = "hidesDockIcon"
    }

    // Both flags below are hand-written accessors rather than `@Published`, and
    // that is load-bearing rather than a style choice.
    //
    // `@Published` notifies on *every* assignment, including one that writes the
    // value the property already holds; a `didSet` guard cannot suppress that,
    // because `didSet` runs after the notification has gone out. `MenuBarExtra`
    // writes its `isInserted` binding back from a KVO observer on the status
    // item *during* a scene update, so that redundant notification invalidated
    // `App.body`, which re-ran the scene update, which wrote the binding again —
    // an unbounded main-thread loop that starved the runloop before the first
    // window was ever presented. Hence the invariant: assigning the value a
    // property already has must not notify observers, must not touch
    // `UserDefaults`, and must not cascade.
    private var storedShowsMenuBarIcon: Bool
    private var storedHidesDockIcon: Bool

    /// Whether the `MenuBarExtra` is inserted in the status bar.
    var showsMenuBarIcon: Bool {
        get { storedShowsMenuBarIcon }
        set {
            guard newValue != storedShowsMenuBarIcon else { return }
            objectWillChange.send()
            storedShowsMenuBarIcon = newValue
            UserDefaults.standard.set(newValue, forKey: Key.showsMenuBarIcon)
            // Removing the last way to reach a Dock-less app would strand it
            // running with no UI at all, so bring the Dock icon back instead.
            if !newValue { hidesDockIcon = false }
        }
    }

    /// Whether the app runs as an accessory (no Dock icon, no app menu).
    var hidesDockIcon: Bool {
        get { storedHidesDockIcon }
        set {
            guard newValue != storedHidesDockIcon else { return }
            objectWillChange.send()
            storedHidesDockIcon = newValue
            UserDefaults.standard.set(newValue, forKey: Key.hidesDockIcon)
            if newValue { showsMenuBarIcon = true }
            applyActivationPolicy()
        }
    }

    /// Mirrors `SMAppService`, which is the source of truth — the user can also
    /// remove the login item from System Settings behind our back.
    @Published private(set) var launchesAtLogin = false

    private init() {
        let defaults = UserDefaults.standard
        defaults.register(defaults: [
            Key.showsMenuBarIcon: true,
            Key.hidesDockIcon: false,
        ])
        // Seeded through the stored values, never the setters: nothing is
        // reconciled yet, there is no NSApp to talk to, and no observer could
        // meaningfully hear about a value the object is being born with.
        storedShowsMenuBarIcon = defaults.bool(forKey: Key.showsMenuBarIcon)
        storedHidesDockIcon = defaults.bool(forKey: Key.hidesDockIcon)
        launchesAtLogin = isLoginItemAvailable && SMAppService.mainApp.status == .enabled
    }

    /// Brings the process's activation policy in line with `hidesDockIcon`.
    /// Called on launch and on every toggle; switching is live, no relaunch.
    func applyActivationPolicy() {
        let policy: NSApplication.ActivationPolicy = hidesDockIcon ? .accessory : .regular
        guard NSApp.activationPolicy() != policy else { return }
        NSApp.setActivationPolicy(policy)

        // Returning to .regular leaves the app inactive with its windows behind
        // everything else, which reads as "the toggle did nothing".
        if policy == .regular {
            NSApp.activate(ignoringOtherApps: true)
        }
    }

    /// `SMAppService.mainApp` needs a real bundle with a bundle identifier;
    /// under `swift run` registration fails, so the toggle is hidden instead of
    /// silently doing nothing. Same reasoning as `UpdateController.isAvailable`.
    var isLoginItemAvailable: Bool {
        Bundle.main.bundleIdentifier != nil && Bundle.main.bundleURL.pathExtension == "app"
    }

    /// Returns the failure so the UI can explain it rather than leave a toggle
    /// flipped back with no reason given.
    @discardableResult
    func setLaunchesAtLogin(_ enabled: Bool) -> Error? {
        guard isLoginItemAvailable else { return nil }
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            launchesAtLogin = SMAppService.mainApp.status == .enabled
            return nil
        } catch {
            // Re-read rather than trusting `enabled`: a failed register may still
            // have left the item in a pending state.
            launchesAtLogin = SMAppService.mainApp.status == .enabled
            return error
        }
    }
}

/// Identifier of the main `Window` scene. SwiftUI copies it onto the `NSWindow`
/// it creates, which is how `AppDelegate` finds that window and nothing else.
enum MainWindow {
    static let id = "main"
}

/// Handles the two things a menu-bar-capable app needs from AppKit and can't
/// express as a `Scene`.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// Earliest hook for the policy: the later it is set, the more chance the
    /// Dock has to draw an icon that then vanishes.
    func applicationWillFinishLaunching(_ notification: Notification) {
        AppSettings.shared.applyActivationPolicy()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        // SwiftUI always opens a `Window` scene at launch. Someone who chose
        // menu-bar-only doesn't want a window in their face every login, and
        // `.defaultLaunchBehavior(.suppressed)` is macOS 15+.
        guard AppSettings.shared.hidesDockIcon else { return }

        // Next runloop pass, not here: at this point SwiftUI has created the
        // window but not shown it, so closing it now is a no-op and it appears
        // a moment later. By the following tick it is on screen and the close
        // sticks.
        DispatchQueue.main.async { self.closeMainWindow() }
    }

    /// Closes *only* the window SwiftUI made for the main scene.
    ///
    /// Matching on the identifier rather than sweeping `NSApp.windows` is
    /// load-bearing: the status item lives in an `NSStatusBarWindow`, which is
    /// not an `NSPanel` and is `isVisible` at launch, so a broader predicate
    /// closes the menu bar icon too — leaving a Dock-less app with no way in.
    private func closeMainWindow() {
        NSApp.windows
            .first { $0.identifier?.rawValue == MainWindow.id }?
            .close()
    }

    /// Closing the window must not quit an app that still lives in the menu bar.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    /// Clicking the Dock icon with no windows open should reopen the main window
    /// rather than do nothing.
    func applicationShouldHandleReopen(
        _ sender: NSApplication,
        hasVisibleWindows flag: Bool
    ) -> Bool {
        true
    }
}
