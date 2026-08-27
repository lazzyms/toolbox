import FirebaseAnalytics
import FirebaseInstallations

@MainActor
final class AnalyticsManager {
    static let shared = AnalyticsManager()
    nonisolated static let defaultEnabled = true

    private var isConfigured = false
    private var isEnabled = false

    private init() {}

    func configure(enabled: Bool) {
        isConfigured = true
        setEnabled(enabled)
        if enabled {
            Installations.installations().installationID { _, _ in }
            log(.appOpened())
        }
    }

    func setEnabled(_ enabled: Bool) {
        guard isConfigured || enabled else {
            isEnabled = false
            return
        }
        let wasEnabled = isEnabled
        isEnabled = enabled
        Analytics.setAnalyticsCollectionEnabled(enabled)
        if !enabled, wasEnabled {
            Installations.installations().delete { _ in }
        }
    }

    func log(_ event: AnalyticsEvent) {
        guard isEnabled else { return }
        Analytics.logEvent(event.name, parameters: event.parameters)
    }
}
